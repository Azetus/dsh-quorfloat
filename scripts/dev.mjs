#!/usr/bin/env node
/**
 * Start dsh against the **distribution form** of this plugin.
 *
 * It stages exactly what `npm publish` would ship — resolved from the manifest's
 * own `files` list and `exports` map — into a throwaway `DSH_HOME`, and runs dsh
 * there. That choice is the point of the script, not an implementation detail:
 *
 * - **It tests what users install.** A checkout has everything, so running from
 *   one cannot notice a file missing from `files`, an `exports` subpath that was
 *   never updated, or a `main` pointing at a path the build no longer emits. Those
 *   are the failures that reach users and nothing else catches, because every unit
 *   test imports the source path directly.
 * - **It exercises resolution from a profile.** dsh resolves a profile bundle by
 *   name from the profile's `node_modules`, which is how the real install works and
 *   is not something a repository path reproduces.
 * - **The profile is generated, not imported.** Nothing in a real `DSH_HOME` is
 *   read except the model credentials, so there is no "install the plugin into your
 *   own profile first" step to get wrong, and no chance of testing a profile that
 *   happens to differ from what is described here. The core bundles are resolved
 *   through the shared `profiles/node_modules` that `dsh` maintains one level up,
 *   so no install runs at all.
 *
 * Two traps it also removes, both of which present as plugin failures:
 *
 * - **A busy port looks like a broken plugin.** `--port` is an app argument, so dsh
 *   does not pick a free port on its own; a leftover process makes the *webserver*
 *   fail, which cascades into `sessionController` never appearing, which leaves
 *   this plugin under "waiting for services" — indistinguishable, in the log, from
 *   a plugin that cannot start. `--port 0` hands the choice to the OS.
 * - **The sidecar path is easy to point at a stale binary.** The host half resolves
 *   `dsh-quorfloat` from a platform package or a release build; here it is pinned to
 *   a copy of this checkout's build, so the plugin and sidecar always match.
 *
 * ## What it does not do
 *
 * No hot reload. HMR replaces modules in Node's loader cache, and it matches
 * changed files against the module URLs that were loaded — which for a staged copy
 * are inside the throwaway home. Re-staging on every build was tried and did not
 * produce a reload, so the option was removed rather than left in place looking
 * like it worked. **The loop is: `npm run build`, then restart.** Startup is a
 * couple of seconds.
 *
 * ## Cross-platform without a shell
 *
 * Everything is `node:fs` — no `cp -R`, no `ln -s`, no `mklink`. A symlink would
 * also need Developer Mode or elevation on Windows, so it would fail on exactly the
 * machines this is meant to help.
 *
 * Usage:
 *   node scripts/dev.mjs [options]
 *   node scripts/dev.mjs --help
 */

import { spawn, spawnSync } from 'node:child_process'
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

/** The executable name, matching what the host half resolves per platform. */
const SIDECAR_NAME = process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat'

/** The font the panel loads; keep in step with `quorfloat/src/fonts.rs`. */
const FONT_NAME = 'NotoSansSC-VF.otf'

/** The licence that must sit beside it. Same directory, so they travel together. */
const FONT_LICENCE = 'OFL.txt'


/** Whether `npm` and `dsh` need a shell to be resolved (they do on Windows). */
const NEEDS_SHELL = process.platform === 'win32'

/**
 * Where the staged home lives.
 *
 * Inside the checkout rather than the system temporary directory: an agent-run
 * shell may not be able to write there, and the failure surfaces later as dsh
 * failing to resolve this very package — which reads as a packaging bug.
 */
const DEV_HOME = join(ROOT, '.dev-home')

/** The manifest, read once: it defines what "distribution form" means here. */
const MANIFEST = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'))

/** Default profile name, generated inside the throwaway home. */
const DEFAULT_PROFILE = 'quorfloat-dev'

/** This package's name, used for the generated profile's bundle and directory. */
const PACKAGE_NAME = MANIFEST.name

/**
 * The bundles a profile needs to be a working Harness with this plugin attached.
 *
 * `dsh-base` provides the host services (the session controller among them) and
 * `dsh-web-app` the HTTP server and browser runtime. Without the second, the app
 * starts with no `sessionController`, this plugin waits for it forever, and the
 * log looks like a plugin fault.
 */
const CORE_BUNDLES = ['@deepseek-ai/dsh-base', '@deepseek-ai/dsh-web-app']

/**
 * The workspace the generated profile starts with.
 *
 * Written by hand because nothing else will do it: the default workspace is created
 * by `@deepseek-ai/dsh-web-app`'s client half on first page load, and this script
 * runs with `--no-open`. Without a registered workspace, `workspaces/list` is empty
 * and every session operation fails at the far end of the protocol — which reads as
 * a plugin fault rather than a missing prerequisite.
 *
 * Fixed rather than random, so a run is reproducible and a stored session still
 * belongs to the workspace that owns it.
 */
const DEV_WORKSPACE_ID = '00000000-0000-4000-8000-000000000001'

/** Parse the options this script owns; anything else is passed through to dsh. */
function parseArgs(argv) {
  const options = {
    profile: DEFAULT_PROFILE,
    port: '0',
    build: true,
    open: false,
    dshHome: undefined,
    passthrough: [],
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    switch (argv[index]) {
      case '--help':
      case '-h':
        options.help = true
        break
      case '--profile':
        options.profile = argv[++index]
        break
      case '--port':
        options.port = argv[++index]
        break
      case '--no-build':
        options.build = false
        break
      case '--open':
        options.open = true
        break
      case '--dsh-home':
        options.dshHome = argv[++index]
        break
      default:
        options.passthrough.push(argv[index])
    }
  }
  return options
}

/** Print usage. */
function usage() {
  process.stdout.write(`Start dsh against the distribution form of dsh-quorfloat.

Usage: node scripts/dev.mjs [options] [-- <extra dsh args>]

Options:
  --profile <name>   profile under DSH_HOME (default: ${DEFAULT_PROFILE})
  --port <port>      listen port; 0 lets the OS pick a free one (default: 0)
  --dsh-home <path>  where to read model credentials from
                     (default: ${join(homedir(), '.dsh')})
  --no-build         skip the pre-flight build
  --open             let dsh open a browser (default: do not)
  -h, --help         this text

Staged into ${DEV_HOME}, rebuilt on every run except for sessions/, which is
carried over so a previous run stays readable. The plugin is staged from the
manifest's own files/exports, so what runs is what would be published — and a
missing entry there fails here rather than at a user's install.

The dsh profile is generated from scratch: core bundles plus this plugin. No
profile in a real DSH_HOME is read or modified.
`)
}

/**
 * Run a command, inheriting stdio so its output is visible.
 *
 * The shell is enabled on Windows only, because `npm` and `dsh` are `.cmd` shims
 * there and `spawnSync` does not resolve them without one. Everywhere else it stays
 * off, so a path containing a space or a shell metacharacter cannot be re-parsed.
 *
 * @returns true when the command reported success.
 */
function run(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, stdio: 'inherit', shell: NEEDS_SHELL })
  return result.status === 0
}

/** Whether `dsh` can be started at all. */
function hasDsh() {
  return spawnSync('dsh', ['--help'], { stdio: 'ignore', shell: NEEDS_SHELL }).error === undefined
}

/**
 * The package-relative paths a published tarball would contain.
 *
 * Read from `files`, which npm treats as the authority, plus `package.json` itself
 * (always included). A missing directory is reported rather than skipped: `files`
 * naming something the build does not produce is precisely the packaging bug this
 * script exists to surface.
 *
 * @returns the paths, or throws with the missing entries named.
 * @throws {Error} when `files` names a path that does not exist.
 */
function publishableEntries() {
  const declared = MANIFEST.files
  if (!Array.isArray(declared) || declared.length === 0) {
    throw new Error('package.json has no `files` list; cannot tell what would be published')
  }
  const missing = declared.filter(entry => !existsSync(join(ROOT, entry)))
  if (missing.length > 0) {
    throw new Error(
      `package.json lists ${missing.map(entry => JSON.stringify(entry)).join(', ')} `
      + 'but the build did not produce it',
    )
  }
  return [...declared, 'package.json']
}

/**
 * Check that every entry point the manifest advertises exists in the staged copy.
 *
 * An `exports` subpath pointing at a file the build stopped emitting is invisible
 * in a checkout — the source path resolves — and fatal for an installed package.
 *
 * @param staged - the staged package directory.
 * @returns the entry points that were checked, for the run summary.
 * @throws {Error} when one is absent.
 */
function verifyEntryPoints(staged) {
  const entryPoints = [MANIFEST.main]
  for (const [subpath, target] of Object.entries(MANIFEST.exports ?? {})) {
    if (typeof target === 'string') entryPoints.push(target)
    else if (target !== null && typeof target === 'object') entryPoints.push(...Object.values(target))
  }
  const unresolved = entryPoints
    .filter(entry => typeof entry === 'string' && entry.startsWith('./'))
    .filter(entry => !existsSync(join(staged, entry)))
  if (unresolved.length > 0) {
    throw new Error(
      `the staged package is missing ${unresolved.map(entry => JSON.stringify(entry)).join(', ')}, `
      + 'which main/exports advertise',
    )
  }
  return entryPoints.filter(entry => typeof entry === 'string')
}

/**
 * Stage the publishable subset of this checkout.
 *
 * `dereference` is left off so a linked file inside `lib/` is copied as a link
 * rather than duplicated.
 *
 * @returns the staged package directory.
 */
function stagePackage() {
  const destination = join(DEV_HOME, 'plugin')
  rmSync(destination, { recursive: true, force: true })
  mkdirSync(destination, { recursive: true })
  for (const entry of publishableEntries()) {
    cpSync(join(ROOT, entry), join(destination, entry), { recursive: true, force: true })
  }
  const entryPoints = verifyEntryPoints(destination)
  return { destination, entryPoints }
}

/**
 * Copy the Rust sidecar next to the staged package.
 *
 * Copied rather than referenced, because the host pins whatever path it is given:
 * pointing at `quorfloat/target/debug` would tie the running sidecar to a directory
 * another `cargo` invocation may be rewriting.
 *
 * A failed refresh is tolerated rather than fatal: Windows refuses to replace a
 * running executable, and the JavaScript half is what changes most of the time.
 *
 * @returns the staged executable's path, or `undefined` when none is built.
 */
function stageSidecar() {
  const candidates = [
    join(ROOT, 'quorfloat', 'target', 'debug', SIDECAR_NAME),
    join(ROOT, 'quorfloat', 'target', 'release', SIDECAR_NAME),
  ]
  const source = candidates.find(candidate => existsSync(candidate))
  if (source === undefined) return undefined

  const directory = join(DEV_HOME, 'sidecar')
  mkdirSync(directory, { recursive: true })
  const destination = join(directory, SIDECAR_NAME)
  if (existsSync(destination)) {
    try {
      rmSync(destination, { force: true })
    } catch {
      process.stdout.write('dev: keeping the previous sidecar copy (it is in use)\n')
      return destination
    }
  }
  cpSync(source, destination, { force: true })
  stageFonts(directory)
  return destination
}

/**
 * Put the font and its licence where the sidecar looks for them.
 *
 * The sidecar resolves `fonts/<file>` next to its own executable, which is exactly the
 * layout the platform package ships (`bin/`, `fonts/`) — so the development harness
 * exercises the same lookup rule as production rather than a special case. Both files
 * are copied together because they ship together: a font without its licence is the
 * packaging mistake this layout is meant to make impossible.
 */
function stageFonts(sidecarDirectory) {
  const directory = join(sidecarDirectory, 'fonts')
  const missing = []
  for (const name of [FONT_NAME, FONT_LICENCE]) {
    const source = join(ROOT, 'quorfloat', 'assets', 'fonts', name)
    const destination = join(directory, name)
    if (!existsSync(source)) {
      missing.push(name)
      // Mirror the repository rather than keeping a copy from an earlier run: a staged
      // font the repository no longer has would let the panel look healthy while a
      // fresh checkout is broken — and it is the very failure this harness exists to
      // show you.
      rmSync(destination, { force: true })
      continue
    }
    mkdirSync(directory, { recursive: true })
    cpSync(source, destination, { force: true })
  }
  if (missing.length > 0) {
    process.stdout.write(
      `dev: missing ${missing.join(' and ')} in quorfloat/assets/fonts/\n`
      + 'dev: the panel will fall back to Latin only — run `npm run fonts` to fetch them\n',
    )
    return
  }
  process.stdout.write(`dev: staged the font and its licence in ${directory}\n`)
}

/** One line describing what the staged sidecar will find. */
function panelFonts(sidecar) {
  if (sidecar === undefined) return '(no panel)'
  const directory = join(dirname(sidecar), 'fonts')
  const font = join(directory, FONT_NAME)
  if (!existsSync(font)) return '(missing: run `npm run fonts`)'
  const megabytes = (statSync(font).size / 1_048_576).toFixed(1)
  const licence = existsSync(join(directory, FONT_LICENCE)) ? `+ ${FONT_LICENCE}` : '(licence missing)'
  return `${font} (${megabytes} MB) ${licence}`
}


/**
 * Write the workspace registry the generated profile starts from.
 *
 * Merges into an existing registry when one survived from a previous run, so the
 * workspace keeps its identity and its session list; a fresh one is created
 * otherwise. `defaultWorkspaceId` is set because that is what makes the workspace
 * the one a new session lands in without the user choosing.
 *
 * @param home - the `DSH_HOME` being generated.
 * @param path - the workspace directory.
 */
function buildWorkspaceRegistry(previous, path, now) {
  const workspaces = { ...(previous?.tables?.workspaces ?? {}) }
  const existing = workspaces[DEV_WORKSPACE_ID]
  workspaces[DEV_WORKSPACE_ID] = {
    path,
    // The title is the directory name, matching what the client half would have
    // produced, so a generated workspace looks the same however it was created.
    title: existing?.title ?? path.split(/[\\/]/).filter(Boolean).pop() ?? 'workspace',
    // Carried over so a stored session still belongs to the workspace that owns it.
    sessionIds: existing?.sessionIds ?? [],
    createdAt: existing?.createdAt ?? now,
    updatedAt: now,
  }
  return {
    unit: { name: 'workspace', version: 2 },
    global: {
      initialized: true,
      workspaceIds: [...new Set([...(previous?.global?.workspaceIds ?? []), DEV_WORKSPACE_ID])],
      archivedSessionIds: previous?.global?.archivedSessionIds ?? [],
      pinnedSessionIds: previous?.global?.pinnedSessionIds ?? [],
      defaultWorkspaceId: DEV_WORKSPACE_ID,
    },
    tables: { workspaces },
  }
}

/** Read the workspace registry, or `undefined` when there is none yet. */
function readWorkspaceRegistry(home) {
  try {
    return JSON.parse(readFileSync(join(home, 'storages', 'workspace.json'), 'utf8'))
  } catch {
    return undefined
  }
}

/** Write the workspace registry, creating the workspace directory with it. */
function writeWorkspaceRegistry(home, path) {
  const file = join(home, 'storages', 'workspace.json')
  const registry = buildWorkspaceRegistry(readWorkspaceRegistry(home), path, new Date().toISOString())
  mkdirSync(dirname(file), { recursive: true })
  writeFileSync(file, `${JSON.stringify(registry, null, 2)}\n`)
  mkdirSync(path, { recursive: true })
}

// Exported for `tests/dev-script.test.mjs`: the registry builder is the one piece of
// this script with behaviour worth pinning, and a script cannot be imported by the
// test runner without running it.
export { buildWorkspaceRegistry, DEV_WORKSPACE_ID }


/**
 * Build the throwaway home: a generated profile with the staged plugin in it.
 *
 * The profile is authored here rather than copied from a real `DSH_HOME`, which
 * removes the one step that used to fail silently. A profile missing
 * `@deepseek-ai/dsh-web-app` yields no `sessionController`, so the app starts and
 * does nothing, and a profile that happens to carry a `link:` dependency makes the
 * plugin load from the checkout instead of from the staged copy — both are
 * indistinguishable in the log from a plugin that cannot start.
 *
 * No install runs. `dsh` keeps the core bundles in the shared
 * `<DSH_HOME>/profiles/node_modules` one level above the profile, and Node resolves
 * upward, so a profile with no `node_modules` of its own still finds them.
 *
 * @param credentialsHome - the real `DSH_HOME` to read model credentials from.
 * @param profile - the profile name to generate.
 * @param workspacePath - the directory the generated workspace points at.
 * @returns the `DSH_HOME` to run against, and what was staged.
 */
function stageHome(credentialsHome, profile, workspacePath) {
  const home = join(DEV_HOME, '.dsh')
  const profileDir = join(home, 'profiles', profile)

  // Session history and the workspace registry survive a restart. They are kept as
  // a pair on purpose: `sessions/` is keyed by workspace path, so carrying one
  // without the other would leave sessions that nothing can reach — they would look
  // lost rather than restored. The workspace record is then re-pointed at this run's
  // path, so a home copied between machines still resolves.
  const durable = join(DEV_HOME, 'durable')
  rmSync(durable, { recursive: true, force: true })
  for (const entry of ['sessions', 'storages']) {
    const existing = join(home, entry)
    if (existsSync(existing)) cpSync(existing, join(durable, entry), { recursive: true, force: true })
  }

  rmSync(home, { recursive: true, force: true })
  mkdirSync(profileDir, { recursive: true })
  for (const entry of ['sessions', 'storages']) {
    const kept = join(durable, entry)
    if (existsSync(kept)) cpSync(kept, join(home, entry), { recursive: true, force: true })
  }
  rmSync(durable, { recursive: true, force: true })

  writeWorkspaceRegistry(home, workspacePath)

  // An empty entry list: the tree is composed from `bundles`, then this file, then
  // any `--patch` overlays. Writing `[]` rather than omitting the file keeps the
  // profile valid without a `dsh plugin` command ever running.
  writeFileSync(join(profileDir, 'cordis.yml'), [
    '# Generated by scripts/dev.mjs — safe to delete.',
    '# The tree is composed from package.json `dsh.profile.bundles`; this file is',
    '# this profile\'s own patch layer and is intentionally empty.',
    '[]',
    '',
  ].join('\n'))

  writeFileSync(join(profileDir, 'package.json'), `${JSON.stringify({
    name: `dsh-profile-${profile}`,
    private: true,
    dsh: { profile: { bundles: [...CORE_BUNDLES, PACKAGE_NAME] } },
  }, null, 2)}\n`)

  // Placed under the profile's own `node_modules` because that is where dsh
  // resolves a bundle *name*, and it is what a real install produces. The profile
  // has no other dependencies, so nothing needs installing.
  const staged = stagePackage()
  const installed = join(profileDir, 'node_modules', PACKAGE_NAME)
  mkdirSync(dirname(installed), { recursive: true })
  cpSync(staged.destination, installed, { recursive: true, force: true })

  // Credentials are the only thing read from a real home, and they are copied
  // rather than linked: this home is rebuilt per run, and a model key that
  // silently stopped working would look like a plugin fault.
  const credentials = join(credentialsHome, '.credentials.yaml')
  if (existsSync(credentials)) {
    cpSync(credentials, join(home, '.credentials.yaml'), { force: true })
  } else {
    process.stdout.write(
      `dev: no credentials at ${credentials}\n`
      + '     the app will start but model turns will fail\n',
    )
  }
  return { home, staged }
}

/** Read the generated profile's bundles, for the summary line. */
function profileBundles(home, profile) {
  try {
    return JSON.parse(readFileSync(join(home, 'profiles', profile, 'package.json'), 'utf8'))
      .dsh?.profile?.bundles ?? []
  } catch {
    return []
  }
}

/**
 * Count the files in a directory tree, for the summary line.
 *
 * The count is the useful number rather than the byte size: a packaging mistake
 * usually shows up as "far fewer files than expected", and it is visible at a
 * glance without comparing sizes across machines.
 *
 * @param directory - the tree to walk.
 * @returns the number of files below it.
 */
function countFiles(directory) {
  let files = 0
  const walk = path => {
    let entries
    try {
      entries = readdirSync(path)
    } catch {
      return
    }
    for (const entry of entries) {
      const child = join(path, entry)
      try {
        if (statSync(child).isDirectory()) walk(child)
        else files += 1
      } catch {
        // A file that vanished mid-walk is not worth failing over.
      }
    }
  }
  walk(directory)
  return files
}

/** Where the sidecar appends its breadcrumbs during a dev run. */
const MARKER_PATH = join(DEV_HOME, 'sidecar.log')

/** Start dsh, and shut it down cleanly on an interrupt. */
function start(options, home, sidecar, staged) {
  const args = ['--profile', options.profile, '--port', options.port]
  if (!options.open) args.push('--no-open')
  args.push(...options.passthrough)

  const env = {
    ...process.env,
    DSH_HOME: home,
    // Pinned so the plugin under development is the sidecar under development.
    // The published package carries no binary yet, so without this the host has
    // nothing to resolve.
    ...(sidecar === undefined ? {} : { DSH_QUORFLOAT_PATH: sidecar }),
    // The panel's only durable record. The host captures the sidecar's stderr and
    // shows it nowhere, so without this file "did the approval reach the panel, and
    // did the click leave it" has no answer after the fact.
    DSH_QUORFLOAT_RUST_MARKER: MARKER_PATH,
    // A deprecation warning from a dependency pollutes every line otherwise.
    NODE_NO_WARNINGS: '1',
  }

  process.stdout.write(`\ndev: dsh --profile ${options.profile} --port ${options.port}\n`
    + `     DSH_HOME=${home}\n`
    + `     staged=${staged.destination} (${countFiles(staged.destination)} files,`
    + ` entry points checked: ${staged.entryPoints.join(', ')})\n`
    + `     sidecar=${sidecar ?? '(not built — no panel)'}\n`
    + `     panel log=${sidecar === undefined ? '(no panel)' : MARKER_PATH}\n`
    + `     panel font=${panelFonts(sidecar)}\n`
    + `     bundles=${profileBundles(home, options.profile).join(', ')}\n`
    + `     workspace=${join(DEV_HOME, 'workspace')}\n\n`)

  const child = spawn('dsh', args, { cwd: ROOT, stdio: 'inherit', env, shell: NEEDS_SHELL })

  // Forward interrupts rather than exiting immediately: dsh needs the chance to
  // stop the sidecar and unregister the hotkey, and an abandoned supervisor is what
  // leaves a port or a hotkey held by a process nobody is watching.
  let stopping = false
  const stop = signal => {
    if (stopping) {
      child.kill('SIGKILL')
      return
    }
    stopping = true
    process.stdout.write(`\ndev: ${signal} received, stopping dsh (again to force)\n`)
    child.kill(signal)
  }
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.on(signal, () => stop(signal))
  }
  child.on('exit', (code, signal) => {
    if (signal !== null) process.stdout.write(`dev: dsh exited on ${signal}\n`)
    process.exit(code ?? (signal === null ? 0 : 1))
  })
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    usage()
    return
  }

  if (!hasDsh()) {
    process.stderr.write('dev: `dsh` is not on PATH\n')
    process.exit(1)
  }

  if (options.build && !run('npm', ['run', 'build'])) {
    process.stderr.write('dev: the build failed; not starting\n')
    process.exit(1)
  }

  const credentialsHome = options.dshHome ?? join(homedir(), '.dsh')
  // Inside the throwaway home so the whole environment stays one deletable
  // directory and session data cannot land in a real project.
  const workspacePath = join(DEV_HOME, 'workspace')
  const { home, staged } = stageHome(credentialsHome, options.profile, workspacePath)
  const sidecar = stageSidecar()
  if (sidecar === undefined) {
    process.stdout.write(
      'dev: the Rust sidecar is not built; the plugin will start without a panel\n'
      + '     build it with: cargo build --manifest-path quorfloat/Cargo.toml\n',
    )
  }
  start(options, home, sidecar, staged)
}

// Only when run as a program: the test runner imports this file for the registry
// builder, and starting dsh as a side effect of that would be absurd.
if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main()
  } catch (error) {
    process.stderr.write(`dev: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exit(1)
  }
}
