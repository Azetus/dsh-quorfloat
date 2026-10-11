// Display labels with a fallback, for fields the host may leave empty.

/**
 * What the panel calls a workspace.
 *
 * The Harness guarantees a non-empty title at registration (it falls back to
 * the directory's basename — `packages/workspace/workspace/src/index.ts`), but
 * a later rename to an empty string is not validated upstream. An empty title
 * therefore shows the path's tail rather than nothing: a row without a name is
 * worse than an approximate one.
 *
 * @param title - what the Harness calls the workspace.
 * @param path - its directory, the honest fallback.
 * @returns the title, or the path's last segment.
 */
export function workspaceLabel(title: string, path: string): string {
  if (title.trim() !== '') return title
  return pathBasename(path)
}

/** The last segment of a path, across separators; never empty. */
function pathBasename(path: string): string {
  const segments = path.split(/[\\/]+/).filter(segment => segment !== '')
  return segments.at(-1) ?? path
}
