// Phosphor Icons Regular SVGs, @phosphor-icons/core 2.1.1
// https://github.com/phosphor-icons/core — MIT License
// Extracted verbatim from the design; these strings are its embedded assets.

// Pruned from the design's asset map because the panel's vocabulary does not use them:
//   chart-pie — replaced by the context ring;
//   clipboard — unused;
//   brain — the thinking menu uses text rows, not icons.
// The permission trio (eye / folder-check / shield) is the design's own mapping for its
// three permission rows; `permissionIcon()` below is the single place that decides which
// permission gets which icon, and both the footer entry and the menu rows go through it.
// Note the one deliberate departure from the design: the design paints the footer pill's
// glyph once as a uniform shield-check, while here the entry carries the glyph of the
// permission actually in force so that the entry and the row it selects agree.
// "corner-down-left" — the ↵ return-arrow mark (U+21B5), copied verbatim from
// Bootstrap Icons' arrow-return-left (filled style, matching this map's Phosphor
// fill convention). Phosphor itself has no corner-arrow icon (verified against
// @phosphor-icons/core 2.1.1 and its current main branch), and the bundled font
// lacks the U+21B5 glyph — so the design's <kbd>↵</kbd> is this shape.
//
// Bootstrap Icons — The MIT License (MIT)
// Copyright (c) 2019-2024 The Bootstrap Authors
// https://github.com/twbs/icons
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
export const ICONS: Record<string, string> = {
  "corner-down-left": "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" fill=\"currentColor\" viewBox=\"0 0 16 16\" aria-hidden=\"true\" focusable=\"false\"><path fill-rule=\"evenodd\" d=\"M14.5 1.5a.5.5 0 0 1 .5.5v4.8a2.5 2.5 0 0 1-2.5 2.5H2.707l3.347 3.346a.5.5 0 0 1-.708.708l-4.2-4.2a.5.5 0 0 1 0-.708l4-4a.5.5 0 1 1 .708.708L2.707 8.3H12.5A1.5 1.5 0 0 0 14 6.8V2a.5.5 0 0 1 .5-.5\"/></svg>",

  "arrow-up": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M205.66,117.66a8,8,0,0,1-11.32,0L136,59.31V216a8,8,0,0,1-16,0V59.31L61.66,117.66a8,8,0,0,1-11.32-11.32l72-72a8,8,0,0,1,11.32,0l72,72A8,8,0,0,1,205.66,117.66Z\"/></svg>",
  "caret-right": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M181.66,133.66l-80,80a8,8,0,0,1-11.32-11.32L164.69,128,90.34,53.66a8,8,0,0,1,11.32-11.32l80,80A8,8,0,0,1,181.66,133.66Z\"/></svg>",
  "check": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M229.66,77.66l-128,128a8,8,0,0,1-11.32,0l-56-56a8,8,0,0,1,11.32-11.32L96,188.69,218.34,66.34a8,8,0,0,1,11.32,11.32Z\"/></svg>",
  "chevron-down": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M213.66,101.66l-80,80a8,8,0,0,1-11.32,0l-80-80A8,8,0,0,1,53.66,90.34L128,164.69l74.34-74.35a8,8,0,0,1,11.32,11.32Z\"/></svg>",
  "database": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M128,24C74.17,24,32,48.6,32,80v96c0,31.4,42.17,56,96,56s96-24.6,96-56V80C224,48.6,181.83,24,128,24Zm80,104c0,9.62-7.88,19.43-21.61,26.92C170.93,163.35,150.19,168,128,168s-42.93-4.65-58.39-13.08C55.88,147.43,48,137.62,48,128V111.36c17.06,15,46.23,24.64,80,24.64s62.94-9.68,80-24.64ZM69.61,53.08C85.07,44.65,105.81,40,128,40s42.93,4.65,58.39,13.08C200.12,60.57,208,70.38,208,80s-7.88,19.43-21.61,26.92C170.93,115.35,150.19,120,128,120s-42.93-4.65-58.39-13.08C55.88,99.43,48,89.62,48,80S55.88,60.57,69.61,53.08ZM186.39,202.92C170.93,211.35,150.19,216,128,216s-42.93-4.65-58.39-13.08C55.88,195.43,48,185.62,48,176V159.36c17.06,15,46.23,24.64,80,24.64s62.94-9.68,80-24.64V176C208,185.62,200.12,195.43,186.39,202.92Z\"/></svg>",

  "eye": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M247.31,124.76c-.35-.79-8.82-19.58-27.65-38.41C194.57,61.26,162.88,48,128,48S61.43,61.26,36.34,86.35C17.51,105.18,9,124,8.69,124.76a8,8,0,0,0,0,6.5c.35.79,8.82,19.57,27.65,38.4C61.43,194.74,93.12,208,128,208s66.57-13.26,91.66-38.34c18.83-18.83,27.3-37.61,27.65-38.4A8,8,0,0,0,247.31,124.76ZM128,192c-30.78,0-57.67-11.19-79.93-33.25A133.47,133.47,0,0,1,25,128,133.33,133.33,0,0,1,48.07,97.25C70.33,75.19,97.22,64,128,64s57.67,11.19,79.93,33.25A133.46,133.46,0,0,1,231.05,128C223.84,141.46,192.43,192,128,192Zm0-112a48,48,0,1,0,48,48A48.05,48.05,0,0,0,128,80Zm0,80a32,32,0,1,1,32-32A32,32,0,0,1,128,160Z\"/></svg>",
  "folder-check": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M216,72H130.67L102.93,51.2a16.12,16.12,0,0,0-9.6-3.2H40A16,16,0,0,0,24,64V200a16,16,0,0,0,16,16H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72Zm0,128H40V64H93.33L123.2,86.4A8,8,0,0,0,128,88h88Z\"/></svg>",
  "shield": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M208,40H48A16,16,0,0,0,32,56v56c0,52.72,25.52,84.67,46.93,102.19,23.06,18.86,46,25.27,47,25.53a8,8,0,0,0,4.2,0c1-.26,23.91-6.67,47-25.53C198.48,196.67,224,164.72,224,112V56A16,16,0,0,0,208,40Zm0,72c0,37.07-13.66,67.16-40.6,89.42A129.3,129.3,0,0,1,128,223.62a128.25,128.25,0,0,1-38.92-21.81C61.82,179.51,48,149.3,48,112l0-56,160,0Z\"/></svg>",
  "folder": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72ZM40,56H92.69l16,16H40ZM216,200H40V88H216Z\"/></svg>",
  "gauge": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M207.06,72.67A111.24,111.24,0,0,0,128,40h-.4C66.07,40.21,16,91,16,153.13V176a16,16,0,0,0,16,16H224a16,16,0,0,0,16-16V152A111.25,111.25,0,0,0,207.06,72.67ZM224,176H119.71l54.76-75.3a8,8,0,0,0-12.94-9.42L99.92,176H32V153.13c0-3.08.15-6.12.43-9.13H56a8,8,0,0,0,0-16H35.27c10.32-38.86,44-68.24,84.73-71.66V80a8,8,0,0,0,16,0V56.33A96.14,96.14,0,0,1,221,128H200a8,8,0,0,0,0,16h23.67c.21,2.65.33,5.31.33,8Z\"/></svg>",
  "message-square": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M216,48H40A16,16,0,0,0,24,64V224a15.84,15.84,0,0,0,9.25,14.5A16.05,16.05,0,0,0,40,240a15.89,15.89,0,0,0,10.25-3.78l.09-.07L83,208H216a16,16,0,0,0,16-16V64A16,16,0,0,0,216,48ZM40,224h0ZM216,192H80a8,8,0,0,0-5.23,1.95L40,224V64H216Z\"/></svg>",
  "pin": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M235.32,81.37,174.63,20.69a16,16,0,0,0-22.63,0L98.37,74.49c-10.66-3.34-35-7.37-60.4,13.14a16,16,0,0,0-1.29,23.78L85,159.71,42.34,202.34a8,8,0,0,0,11.32,11.32L96.29,171l48.29,48.29A16,16,0,0,0,155.9,224c.38,0,.75,0,1.13,0a15.93,15.93,0,0,0,11.64-6.33c19.64-26.1,17.75-47.32,13.19-60L235.33,104A16,16,0,0,0,235.32,81.37ZM224,92.69h0l-57.27,57.46a8,8,0,0,0-1.49,9.22c9.46,18.93-1.8,38.59-9.34,48.62L48,100.08c12.08-9.74,23.64-12.31,32.48-12.31A40.13,40.13,0,0,1,96.81,91a8,8,0,0,0,9.25-1.51L163.32,32,224,92.68Z\"/></svg>",
  "pin-off": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M53.92,34.62A8,8,0,1,0,42.08,45.38L67.37,73.2A69.82,69.82,0,0,0,38,87.63a16,16,0,0,0-1.29,23.78L85,159.71,42.34,202.34a8,8,0,0,0,11.32,11.32L96.29,171l48.29,48.29A16,16,0,0,0,155.9,224c.38,0,.75,0,1.13,0a15.93,15.93,0,0,0,11.64-6.33,89.75,89.75,0,0,0,11.58-20.27l21.84,24a8,8,0,1,0,11.84-10.76ZM155.9,208,48,100.08C58.23,91.83,69.2,87.72,80.66,87.81l87.16,95.88C165.59,193.56,160.24,202.23,155.9,208Zm79.42-104-44.64,44.79a8,8,0,1,1-11.33-11.3L224,92.7,163.32,32,122.1,73.35a8,8,0,0,1-11.33-11.29L152,20.7a16,16,0,0,1,22.63,0l60.69,60.68A16,16,0,0,1,235.32,104Z\"/></svg>",
  "plus": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z\"/></svg>",
  "search": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M229.66,218.34l-50.07-50.06a88.11,88.11,0,1,0-11.31,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z\"/></svg>",
  "settings-2": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M128,80a48,48,0,1,0,48,48A48.05,48.05,0,0,0,128,80Zm0,80a32,32,0,1,1,32-32A32,32,0,0,1,128,160Zm109.94-52.79a8,8,0,0,0-3.89-5.4l-29.83-17-.12-33.62a8,8,0,0,0-2.83-6.08,111.91,111.91,0,0,0-36.72-20.67,8,8,0,0,0-6.46.59L128,41.85,97.88,25a8,8,0,0,0-6.47-.6A112.1,112.1,0,0,0,54.73,45.15a8,8,0,0,0-2.83,6.07l-.15,33.65-29.83,17a8,8,0,0,0-3.89,5.4,106.47,106.47,0,0,0,0,41.56,8,8,0,0,0,3.89,5.4l29.83,17,.12,33.62a8,8,0,0,0,2.83,6.08,111.91,111.91,0,0,0,36.72,20.67,8,8,0,0,0,6.46-.59L128,214.15,158.12,231a7.91,7.91,0,0,0,3.9,1,8.09,8.09,0,0,0,2.57-.42,112.1,112.1,0,0,0,36.68-20.73,8,8,0,0,0,2.83-6.07l.15-33.65,29.83-17a8,8,0,0,0,3.89-5.4A106.47,106.47,0,0,0,237.94,107.21Zm-15,34.91-28.57,16.25a8,8,0,0,0-3,3c-.58,1-1.19,2.06-1.81,3.06a7.94,7.94,0,0,0-1.22,4.21l-.15,32.25a95.89,95.89,0,0,1-25.37,14.3L134,199.13a8,8,0,0,0-3.91-1h-.19c-1.21,0-2.43,0-3.64,0a8.08,8.08,0,0,0-4.1,1l-28.84,16.1A96,96,0,0,1,67.88,201l-.11-32.2a8,8,0,0,0-1.22-4.22c-.62-1-1.23-2-1.8-3.06a8.09,8.09,0,0,0-3-3.06l-28.6-16.29a90.49,90.49,0,0,1,0-28.26L61.67,97.63a8,8,0,0,0,3-3c.58-1,1.19-2.06,1.81-3.06a7.94,7.94,0,0,0,1.22-4.21l.15-32.25a95.89,95.89,0,0,1,25.37-14.3L122,56.87a8,8,0,0,0,4.1,1c1.21,0,2.43,0,3.64,0a8.08,8.08,0,0,0,4.1-1l28.84-16.1A96,96,0,0,1,188.12,55l.11,32.2a8,8,0,0,0,1.22,4.22c.62,1,1.23,2,1.8,3.06a8.09,8.09,0,0,0,3,3.06l28.6,16.29A90.49,90.49,0,0,1,222.9,142.12Z\"/></svg>",
  "shield-check": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M208,40H48A16,16,0,0,0,32,56v56c0,52.72,25.52,84.67,46.93,102.19,23.06,18.86,46,25.26,47,25.53a8,8,0,0,0,4.2,0c1-.27,23.91-6.67,47-25.53C198.48,196.67,224,164.72,224,112V56A16,16,0,0,0,208,40Zm0,72c0,37.07-13.66,67.16-40.6,89.42A129.3,129.3,0,0,1,128,223.62a128.25,128.25,0,0,1-38.92-21.81C61.82,179.51,48,149.3,48,112l0-56,160,0ZM82.34,141.66a8,8,0,0,1,11.32-11.32L112,148.69l50.34-50.35a8,8,0,0,1,11.32,11.32l-56,56a8,8,0,0,1-11.32,0Z\"/></svg>",
  "square": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M208,32H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32Zm0,176H48V48H208V208Z\"/></svg>",
  "x": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" fill=\"currentColor\" width=\"16\" height=\"16\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M205.66,194.34a8,8,0,0,1-11.32,11.32L128,139.31,61.66,205.66a8,8,0,0,1-11.32-11.32L116.69,128,50.34,61.66A8,8,0,0,1,61.66,50.34L128,116.69l66.34-66.35a8,8,0,0,1,11.32,11.32L139.31,128Z\"/></svg>",
}

/** One asset, split into what React needs to build the `<svg>` itself: the panel's CSS
 *  positions the svg element (`.q-option > svg`, `.q-picker > svg:first-child`), so the
 *  element has to be real and only its *children* may come from the asset string. */
export interface IconParts {
  /** The asset's viewBox, e.g. `0 0 256 256`. */
  readonly viewBox: string
  /** The asset's width and height attributes. */
  readonly size: { readonly width: string; readonly height: string }
  /** The markup *inside* the `<svg>`: paths, parsed once per asset. */
  readonly inner: string
}

const PARTS = new Map<string, IconParts | null>()

/**
 * Split one embedded asset into its `<svg>` attributes and its inner markup.
 *
 * Trusted input (assets copied from the design at build time), parsed once. The old
 * imperative renderer built the whole element from the string; React cannot, so the
 * string is taken apart here and the element itself is rendered by `Icon`.
 *
 * @param name - the asset key (see the ICONS map).
 * @returns the parts, or null for a name this build does not carry.
 */
export function iconParts(name: string): IconParts | null {
  const cached = PARTS.get(name)
  if (cached !== undefined) return cached
  const source = ICONS[name]
  if (source === undefined) {
    PARTS.set(name, null)
    return null
  }
  const host = document.createElement('div')
  host.innerHTML = source.trim()
  const svg = host.firstElementChild
  const parts = svg instanceof SVGElement
    ? {
      viewBox: svg.getAttribute('viewBox') ?? '0 0 16 16',
      size: {
        width: svg.getAttribute('width') ?? '16',
        height: svg.getAttribute('height') ?? '16',
      },
      inner: svg.innerHTML,
    }
    : null
  PARTS.set(name, parts)
  return parts
}

/** The permission trio's glyphs, keyed by the preset's machine value. */
const PERMISSION_GLYPHS = new Map<string, string>([
  ['read-only', 'eye'],
  ['workspace-write', 'folder-check'],
  ['danger-full-access', 'shield'],
])

/**
 * Pick the glyph for one permission preset.
 *
 * The design keys its three rows by the Chinese label (只读 / 当前工作区 / 完整权限), but the
 * label is the host's to rewrite — the adapter translates the built-in one and a configured
 * preset keeps whatever the user called it — so the panel keys by the **value** instead, the
 * same stable keys Harness's own picker uses
 * (`packages/client/ui-primitives/src/PermissionIcon.tsx`).
 *
 * Everywhere a permission is named — the menu rows and the footer entry that opens them —
 * goes through here, so one permission never shows two glyphs.
 *
 * A preset outside the trio falls back to the uniform shield-check: an unrecognised name
 * must not borrow another option's meaning. `null` (no catalog yet, no session attached)
 * falls back the same way.
 *
 * @param value - the preset's machine value from `session/options`, or null if unknown.
 * @returns the asset key for that permission.
 */
export function permissionIcon(value: string | null): string {
  return PERMISSION_GLYPHS.get(value ?? '') ?? 'shield-check'
}

/** The context ring: a 14px viewBox, 2px stroke, radius 5.5, from twelve o'clock
 *  (the design's ContextMeter equivalent). */
export function ring(percent: number): string {
  const radius = 5.5
  const circumference = 2 * Math.PI * radius
  const clamped = Math.max(0, Math.min(1, percent / 100))
  // 0% draws no arc at all: a dot at twelve o'clock would read as "used a little".
  const arc = clamped === 0
    ? ''
    : `<circle cx="7" cy="7" r="${radius}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-dasharray="${(clamped * circumference).toFixed(3)} ${circumference.toFixed(3)}" transform="rotate(-90 7 7)"/>`
  return `<svg viewBox="0 0 14 14" width="13" height="13" aria-hidden="true"><circle cx="7" cy="7" r="${radius}" fill="none" stroke="currentColor" stroke-opacity="0.35" stroke-width="2"/>${arc}</svg>`
}
