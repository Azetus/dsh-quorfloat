/**
 * Tests for the scripted peer used to verify the plugin inside a real dsh.
 *
 * The peer is itself a protocol participant, so it can be verified without a
 * harness: this file plays the host, speaks the documented handshake, and checks
 * that the peer introduces itself correctly, keeps stdout frame-pure, stays
 * alive while idle, and exits when asked — the four properties the plugin's
 * channel relies on.
 */

import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { cleanupDir, loadModule, repoRoot, scratchDir, waitFor } from './helpers.mjs'

const { FrameDecoder, encodeFrame } = await loadModule('protocol.js')
const peerPath = join(repoRoot, 'scripts/peer-probe.mjs')

/** Spawn the peer and collect its frames and stderr. */
function startPeer({ mode = 'handshake', reportPath, env = {} } = {}) {
  const child = spawn(process.execPath, [peerPath], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: {
      ...process.env,
      DSH_QUORFLOAT_PEER_MODE: mode,
      ...(reportPath === undefined ? {} : { DSH_QUORFLOAT_PEER_REPORT: reportPath }),
      ...env,
    },
  })
  const decoder = new FrameDecoder()
  const frames = []
  const rejections = []
  const stderr = []
  let text = ''
  child.stdout.on('data', chunk => {
    // stdout must be frame-pure: every byte has to decode as one JSON-RPC frame.
    for (const frame of decoder.push(chunk)) {
      if (frame.ok) frames.push(frame)
      else rejections.push(frame.rejection)
    }
  })
  child.stderr.on('data', chunk => {
    text += chunk.toString()
  })
  const exited = new Promise(resolve => child.on('exit', (code, signal) => resolve({ code, signal })))
  return {
    child,
    frames,
    rejections,
    stderr,
    text: () => text,
    exited,
    send: message => child.stdin.write(encodeFrame(message)),
    close: async () => {
      child.stdin.end()
      return await exited
    },
  }
}

/** The next request frame with a given method, waiting for it. */
async function waitForRequest(peer, method) {
  return await waitFor(`the peer to call ${method}`, async () =>
    peer.frames.find(frame => frame.ok && frame.kind === 'request' && frame.message.method === method))
}

test('the peer introduces itself with the documented handshake', async () => {
  const peer = startPeer()
  try {
    const hello = await waitForRequest(peer, 'hello')
    assert.equal(hello.message.params.protocol, 'quorfloat/1')
    assert.equal(hello.message.params.platform, process.platform)
    assert.ok(Array.isArray(hello.message.params.capabilities))
    assert.equal(typeof hello.message.params.hotkey.registered, 'boolean')
    assert.deepEqual(peer.rejections, [], 'nothing but protocol frames may reach stdout')
  } finally {
    await peer.close()
  }
})

test('the peer reports the host handshake and stays alive when idle', async () => {
  const dir = scratchDir()
  const reportPath = join(dir, 'report.json')
  const peer = startPeer({ reportPath })
  try {
    const hello = await waitForRequest(peer, 'hello')
    peer.send({ jsonrpc: '2.0', id: hello.message.id, result: { protocol: 'quorfloat/1', hostVersion: 'test', sessionId: 'test', capabilities: [], window: {}, hotkey: 'Alt+Space' } })
    peer.send({ jsonrpc: '2.0', method: 'ready', params: { protocol: 'quorfloat/1', config: { hotkey: 'Alt+Space' } } })
    await waitFor('the peer to note the handshake', async () => peer.text().includes('handshake answered'))
    // It must keep running: a peer that exits after the handshake would make the
    // plugin look dead, and "hold" mode exists exactly to prove it does not.
    assert.equal(peer.child.exitCode, null, 'an idle peer stays alive')
  } finally {
    const result = await peer.close()
    assert.equal(result.code, 0, 'stdin EOF is a clean exit')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    assert.equal(report.ready, true, 'the ready notification was observed')
    assert.equal(report.handshake.protocol, 'quorfloat/1')
    assert.ok(report.stderr.some(line => line.includes('handshake answered')), 'stderr carries the narration')
    cleanupDir(dir)
  }
})

test('the peer answers a ping so host liveness detection works', async () => {
  const peer = startPeer()
  try {
    const hello = await waitForRequest(peer, 'hello')
    peer.send({ jsonrpc: '2.0', id: hello.message.id, result: { protocol: 'quorfloat/1' } })
    peer.send({ jsonrpc: '2.0', id: 9001, method: 'ping', params: {} })
    const pong = await waitFor('a pong', async () =>
      peer.frames.find(frame => frame.ok && frame.kind === 'success' && frame.message.id === 9001))
    assert.equal(typeof pong.message.result.pong, 'number')
  } finally {
    await peer.close()
  }
})

test('a shutdown request makes the peer exit, reporting what it saw', async () => {
  const dir = scratchDir()
  const reportPath = join(dir, 'report.json')
  const peer = startPeer({ mode: 'hold', reportPath })
  try {
    const hello = await waitForRequest(peer, 'hello')
    peer.send({ jsonrpc: '2.0', id: hello.message.id, result: { protocol: 'quorfloat/1' } })
    peer.send({ jsonrpc: '2.0', id: 9002, method: 'shutdown', params: { reason: 'host-closing' } })
    const result = await peer.exited
    assert.equal(result.code, 0, 'a cooperative peer exits without a signal')
    const report = JSON.parse(readFileSync(reportPath, 'utf8'))
    assert.equal(report.mode, 'hold')
    assert.ok(report.stderr.some(line => line.includes('host asked us to shut down')))
    cleanupDir(dir)
  } finally {
    await peer.close()
  }
})

test('an unknown host request is refused instead of ignored', async () => {
  const peer = startPeer()
  try {
    const hello = await waitForRequest(peer, 'hello')
    peer.send({ jsonrpc: '2.0', id: hello.message.id, result: { protocol: 'quorfloat/1' } })
    peer.send({ jsonrpc: '2.0', id: 9003, method: 'no/such/method', params: {} })
    const refused = await waitFor('a method-not-found response', async () =>
      peer.frames.find(frame => frame.ok && frame.kind === 'failure' && frame.message.id === 9003))
    assert.equal(refused.message.error.code, -32601)
  } finally {
    await peer.close()
  }
})

test('a malformed host frame does not break the peer', async () => {
  const peer = startPeer()
  try {
    const hello = await waitForRequest(peer, 'hello')
    // Garbage, plus a frame split across two writes: both are things a host can
    // legitimately produce, and neither may cost the peer a later frame.
    peer.child.stdin.write('not json at all\n')
    const journal = encodeFrame({ jsonrpc: '2.0', method: 'host/heartbeat', params: { t: 1 } })
    peer.child.stdin.write(journal.slice(0, 12))
    peer.child.stdin.write(journal.slice(12))
    peer.send({ jsonrpc: '2.0', id: hello.message.id, result: { protocol: 'quorfloat/1' } })
    await waitFor('the peer to note the bad frame', async () => peer.text().includes('could not parse'))
    peer.send({ jsonrpc: '2.0', id: 9004, method: 'ping', params: {} })
    await waitFor('a pong after the garbage', async () =>
      peer.frames.find(frame => frame.ok && frame.kind === 'success' && frame.message.id === 9004))
  } finally {
    await peer.close()
  }
})
