#!/usr/bin/env node
/**
 * Run one approval-authority scenario against a real dsh.
 *
 * The authority decision depends on two facts that can only be observed at
 * runtime — whether a Harness interface is visible and focused, and whether the
 * floating panel is on screen. This script sets up one combination, drives a turn
 * that requests a sandbox escalation, and reports who answered the approval.
 *
 * It runs `dsh` against `DSH_HOME` exactly as configured in the environment, so
 * point that at a throwaway home. Nothing here writes to the user's real profile.
 *
 * Usage:
 *   node scripts/scenario.mjs <name> <spec>
 *   spec := ui:<surface>:<visible|hidden>:<focused|unfocused>[,panel:<visible|hidden>]
 *
 * Examples:
 *   node scripts/scenario.mjs v3 ui:web:visible:focused,panel:visible
 *   node scripts/scenario.mjs v5b panel:visible
 *   node scripts/scenario.mjs v5a
 *
 * Exit code is 0 when the run produced a verdict, 1 when the scenario could not
 * be evaluated (for example the model never asked to escalate).
 */

import { spawn } from 'node:child_process'
import { existsSync, readFileSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))
const name = process.argv[2] ?? 'scenario'
const spec = process.argv[3] ?? ''
const reportPath = process.env['DSH_QUORFLOAT_PEER_REPORT'] ?? `/tmp/qf-${name}.json`
const workspaceId = process.env['DSH_QUORFLOAT_PEER_WORKSPACE'] ?? ''
const runMs = Number(process.env['QF_SCENARIO_MS'] ?? 120_000)

if (spec === '') {
  console.error('usage: node scripts/scenario.mjs <name> <ui:surface:visible:focused,panel:visible>')
  process.exit(2)
}

/** Translate the scenario spec into the probe's two announcement knobs. */
function probeEnv() {
  const env = {
    DSH_QUORFLOAT_PEER_MODE: 'auto',
    DSH_QUORFLOAT_PEER_REPORT: reportPath,
    DSH_QUORFLOAT_PEER_PROMPT: process.env['DSH_QUORFLOAT_PEER_PROMPT']
      ?? 'Run this bash command with sandbox_permissions="danger-full-access" and '
        + `justification="${name} authority scenario": echo ${name} > /tmp/qf-scenario-out.txt`,
  }
  for (const part of spec.split(',')) {
    const [key, ...rest] = part.split(':')
    if (key === 'panel') {
      if (rest[0] === 'visible') env['DSH_QUORFLOAT_PEER_PANEL_VISIBLE'] = '1'
      continue
    }
    if (key === 'ui') {
      const [surface, visibility, focus] = rest
      env['DSH_QUORFLOAT_PEER_UI'] = `${surface}:${visibility}:${focus}`
      continue
    }
    console.error(`scenario: unknown spec part ${JSON.stringify(part)}`)
    process.exit(2)
  }
  if (workspaceId !== '') env['DSH_QUORFLOAT_PEER_WORKSPACE'] = workspaceId
  return env
}

rmSync(reportPath, { force: true })

const child = spawn('dsh', ['--profile', process.env['QF_PROFILE'] ?? 'quorfloat-dev', '--no-open'], {
  cwd: ROOT,
  stdio: ['ignore', 'pipe', 'pipe'],
  env: { ...process.env, DSH_QUORFLOAT_PATH: join(ROOT, 'scripts', 'peer-probe.mjs'), ...probeEnv() },
})

const diagnostics = []
child.stderr.on('data', chunk => {
  for (const line of String(chunk).split('\n')) {
    if (line.includes('[qf-diag]')) diagnostics.push(line.trim())
  }
})
child.stdout.on('data', () => {})
child.on('error', error => {
  console.error('scenario: could not start dsh:', error.message)
  process.exit(1)
})

const finished = new Promise(resolve => child.on('close', resolve))
const timer = setTimeout(() => child.kill('SIGTERM'), runMs)
await finished
clearTimeout(timer)
await new Promise(resolve => setTimeout(resolve, 2000))

if (!existsSync(reportPath)) {
  console.error(`scenario ${name}: the probe wrote no report; it likely never reached a session`)
  process.exit(1)
}
const report = JSON.parse(readFileSync(reportPath, 'utf8'))
const kinds = {}
for (const event of report.events ?? []) {
  if (event.kind === 'event') kinds[event.type] = (kinds[event.type] ?? 0) + 1
}

const asked = kinds['approval/asked'] ?? 0
const decided = kinds['approval/decided'] ?? 0
const answered = (report.answered ?? []).length
const hints = (report.hints ?? []).length

console.log(`\n=== scenario ${name} (spec: ${spec}) ===`)
console.log('announced      :', JSON.stringify(report.announced))
console.log('approval/asked :', asked, '| approval/decided:', decided)
console.log('panel answered :', answered, '| hints sent:', hints)
for (const hint of report.hints ?? []) console.log('  hint:', JSON.stringify(hint))
const conclusion = report.conclusion ?? {}
console.log('turn end       :', JSON.stringify(conclusion.turnEnd ?? null))
console.log('approval results:', JSON.stringify(conclusion.approvalOutcomes ?? []))
if (diagnostics.length > 0) {
  console.log('decisions      :')
  for (const line of diagnostics) console.log('  ' + line)
}
for (const line of report.stderr ?? []) {
  if (line.includes('tool/call') || line.includes('belongs to the Harness window')) console.log('probe          :', line.trim())
}

if (asked === 0) {
  console.log('\nNOT EVALUATED: the model never requested escalation, so no approval was raised.')
  console.log('Re-run; the escalation request is up to the model. The tool/call lines above show what it did.')
  process.exit(1)
}

// Who answered is the whole point: a panel answer proves the panel claimed it,
// and an approval that was asked but never decided proves the panel deferred.
const who = answered > 0 ? 'panel' : 'harness'
console.log(`\nVERDICT: ${who} answered the approval`)
process.exit(0)
