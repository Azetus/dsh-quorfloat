//! Fonts: making the panel able to draw more than Latin.
//!
//! egui's built-in set is Latin (Ubuntu-Light) plus a small emoji subset, so every
//! Chinese string this panel writes would otherwise render as the box epaint
//! substitutes for a character no font in the family can draw.
//!
//! ## Why a bundled font instead of the system's
//!
//! Reading a system font is cheaper to *write* and worse to *ship*, and this was
//! measured on a real machine rather than argued (`docs/prototype.md` §22):
//!
//! | approach | resident memory | coverage |
//! |---|---|---|
//! | the locale-driven crate call (six families) | +428 MB | zh, ja, ko, ar, he |
//! | one system family (`STHeiti`) | +26 MB | zh, ja |
//! | **this module** (one bundled file) | **+10 MB** | zh, ja, el, cyrillic |
//!
//! Beyond memory, system fonts bring their own surprises: macOS 15+ keeps PingFang in
//! an on-demand asset directory, and `.ttc` collections are loaded at face index 0 —
//! which for PingFang is the *Hong Kong* face. A file we ship has neither problem, and
//! the same bytes render the same way on all three platforms.
//!
//! ## Where the file lives, and what happens when it is not there
//!
//! The platform package carries `fonts/NotoSansSC-VF.otf` beside `bin/dsh-quorfloat`,
//! and the file is found by walking a short list of candidate paths. Reading it at
//! runtime rather than compiling it in costs one failure mode — a package that is
//! missing a file — and the answer to that is not to pretend it cannot happen but to
//! say so: [`FontStatus`] carries what was tried, the marker file records it, and the
//! panel shows the user a line it can still draw.
//!
//! `OFL.txt` travels in the same directory. That is not decoration: the OFL requires
//! the licence to accompany every copy, and the copyright line lives in the font's name
//! table, which a user cannot read out of a file they do not know about.
//!
//! ## The one implementation detail that matters
//!
//! The bytes are leaked to `'static` and handed to egui as a borrow. Measured: the
//! owned path (`FontData::from_owned`) keeps a second copy of the whole file, costing
//! an extra 14 MB here, and every further weight instance would pay it again.
//!
//! ## What it does not cover
//!
//! Korean, Arabic, Hebrew, Thai, Devanagari and colour emoji. That is a deliberate
//! limit rather than an oversight: each would cost another file, and a panel that
//! silently draws boxes is worse than one that says what it can do.

use std::path::{Path, PathBuf};

use eframe::egui;

/// File name of the bundled font, inside a `fonts/` directory.
pub const FONT_FILE: &str = "NotoSansSC-VF.otf";

/// Environment override naming the font outright.
///
/// The development harness uses it, because the staged sidecar lives in a throwaway
/// directory whose layout is not the shipped one.
pub const FONT_PATH_ENV: &str = "DSH_QUORFLOAT_FONT_PATH";

/// Name this font is registered under inside egui.
const FAMILY: &str = "noto-sans-sc";

/// The panel's own text, used to check that the font actually arrived.
///
/// Taken from the UI rather than invented, so the check fails when the *panel* cannot
/// be read — which is the failure a user would actually notice.
const PANEL_SAMPLE: &str = "允许一次 拒绝";

/// What happened when the fonts were installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontStatus {
    /// The bundled font is in use.
    Loaded {
        /// The file that was read.
        path: PathBuf,
        /// How many bytes were handed to egui.
        bytes: usize,
    },
    /// No font file was found, so only the built-in Latin set will render.
    Missing {
        /// Every path that was tried, for the log line that has to explain this.
        searched: Vec<PathBuf>,
    },
}

impl FontStatus {
    /// Whether a bundled font was loaded.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded { .. })
    }

    /// One line for the log and the marker file.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Loaded { path, bytes } => {
                format!("fonts loaded {FAMILY} from {} ({bytes} bytes)", path.display())
            }
            Self::Missing { searched } => {
                let tried = searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fonts missing: no {FONT_FILE} in [{tried}]")
            }
        }
    }

    /// What the panel should tell the user, when it cannot draw its own text.
    ///
    /// `None` when there is nothing to warn about: a warning that is always present is
    /// a warning nobody reads.
    ///
    /// The wording is deliberately not Chinese. This line exists for the case where the
    /// panel cannot draw Chinese, so a Chinese warning would be the one string the user
    /// could not read.
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Loaded { .. } => None,
            Self::Missing { .. } => Some(format!(
                "No CJK font found ({FONT_FILE}): Chinese text will not render. Run `npm run fonts`.",
            )),
        }
    }
}

/// Paths to try, in order.
///
/// The order is the whole contract: an explicit override wins over anything shipped,
/// and the shipped layout (`<package>/fonts/`) wins over a flat directory next to the
/// binary, which is how the development harness stages it.
///
/// @param exe_dir - directory holding this executable, when it can be determined.
/// @param override_path - the value of [`FONT_PATH_ENV`], when set and non-empty.
/// @returns the candidate paths, duplicates removed.
#[must_use]
pub fn candidate_paths(exe_dir: Option<&Path>, override_path: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    if let Some(path) = override_path {
        push(path.to_path_buf());
    }
    if let Some(directory) = exe_dir {
        // The platform package: `bin/dsh-quorfloat` with `fonts/` beside `bin/`.
        if let Some(root) = directory.parent() {
            push(root.join("fonts").join(FONT_FILE));
        }
        // A flat layout, which is what the development harness stages.
        push(directory.join("fonts").join(FONT_FILE));
    }
    candidates
}

/// Find the font file, if it is anywhere it is expected to be.
///
/// @param exe_dir - directory holding this executable, when it can be determined.
/// @returns the first candidate that exists, and the full list that was tried.
#[must_use]
pub fn locate(exe_dir: Option<&Path>) -> (Option<PathBuf>, Vec<PathBuf>) {
    let override_path = std::env::var_os(FONT_PATH_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    let candidates = candidate_paths(exe_dir, override_path.as_deref());
    let found = candidates.iter().find(|path| path.is_file()).cloned();
    (found, candidates)
}

/// The directory holding this executable.
#[must_use]
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|path| path.parent().map(Path::to_path_buf))
}

/// Install the bundled font as the lowest-priority fallback.
///
/// Lowest priority is the point: Latin text keeps egui's own font, and this one is
/// consulted only for characters the built-in set does not have.
///
/// @param ctx - the render context. Fonts must be in place before the first frame, or
///   that frame lays text out with a set that is about to change.
/// @returns what was loaded, or why nothing was.
pub fn install(ctx: &egui::Context) -> FontStatus {
    let (found, searched) = locate(exe_dir().as_deref());
    let Some(path) = found else {
        return FontStatus::Missing { searched };
    };
    install_from(ctx, &path)
}

/// Install a specific font file.
///
/// Separate from [`install`] so the operation can be exercised against a known file —
/// the end-to-end test points at the asset in the repository, which no staged layout
/// puts next to the test binary.
///
/// @param ctx - the render context.
/// @param path - the font file to load.
/// @returns what was loaded, or why nothing was.
pub fn install_from(ctx: &egui::Context, path: &Path) -> FontStatus {
    let Ok(bytes) = std::fs::read(path) else {
        // Unreadable is as good as absent, and the fallback is the same: keep running
        // with Latin only. The path is in the status, so the log says which file it was.
        return FontStatus::Missing { searched: vec![path.to_path_buf()] };
    };
    let size = bytes.len();
    // Leaked on purpose: the font lives as long as the process does, and a `'static`
    // slice lets every weight share one copy of the bytes instead of one each.
    let shared: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    ctx.add_font(egui::epaint::text::FontInsert {
        name: FAMILY.to_owned(),
        data: egui::FontData::from_static(shared),
        families: vec![
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
        ],
    });
    FontStatus::Loaded { path: path.to_path_buf(), bytes: size }
}

/// Whether the panel can currently draw its own text.
///
/// Measured through egui's own glyph lookup, so it answers the question the renderer
/// will ask — glyphs, or a box. Installing fonts is a *claim* about a file; this is the
/// fact, which is why both are reported.
///
/// **Must be called after a pass.** There is no font set until the context has run, and
/// calling it earlier panics inside egui.
///
/// @param ctx - the render context.
/// @returns whether every character of the panel's own text has a glyph.
#[must_use]
pub fn covers_panel_text(ctx: &egui::Context) -> bool {
    ctx.fonts_mut(|fonts| fonts.has_glyphs(&egui::FontId::proportional(14.0), PANEL_SAMPLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file path that exists, for tests that only care about existence.
    fn existing_file(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the directory");
        std::fs::write(&path, b"not really a font").expect("write the file");
        path
    }

    /// A scratch directory that cleans itself up.
    fn scratch(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("quorfloat-fonts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    /// Run one pass, which is what creates a font set at all.
    fn pass(ctx: &egui::Context) {
        // The output carries texture deltas that epaint insists be handled or
        // explicitly discarded; a test that ignores them panics on drop.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
    }

    #[test]
    fn an_override_beats_everything_shipped() {
        let candidates = candidate_paths(Some(Path::new("/app/bin")), Some(Path::new("/custom/font.otf")));
        assert_eq!(candidates[0], PathBuf::from("/custom/font.otf"));
        assert!(candidates.contains(&PathBuf::from("/app/fonts").join(FONT_FILE)));
        assert!(candidates.contains(&PathBuf::from("/app/bin/fonts").join(FONT_FILE)));
    }

    #[test]
    fn a_missing_override_or_executable_still_produces_a_list() {
        assert_eq!(candidate_paths(None, None), Vec::<PathBuf>::new());
        assert_eq!(
            candidate_paths(None, Some(Path::new("/only.otf"))),
            vec![PathBuf::from("/only.otf")],
        );
    }

    #[test]
    fn duplicates_are_removed_so_the_search_list_stays_readable() {
        // With the executable in the package root, the package layout and the flat
        // layout name the same file — and this list is printed in a log line.
        let candidates = candidate_paths(Some(Path::new("/app")), None);
        let mut unique = candidates.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), candidates.len(), "{candidates:?}");
    }

    #[test]
    fn the_first_existing_candidate_wins() {
        let directory = scratch("locate");
        let flat = existing_file(&directory.join("bin/fonts"), FONT_FILE);
        let found = candidate_paths(Some(&directory.join("bin")), None)
            .into_iter()
            .find(|path| path.is_file());
        assert_eq!(found, Some(flat));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_font_says_what_it_looked_for_and_warns_in_a_readable_language() {
        let (found, searched) = locate(Some(Path::new("/definitely/not/here")));
        assert!(found.is_none());
        assert!(!searched.is_empty());
        let status = FontStatus::Missing { searched };
        assert!(!status.is_loaded());
        assert!(status.describe().contains(FONT_FILE));
        let warning = status.warning().expect("a panel that cannot draw Chinese has to say so");
        // The one line the user would read in this situation must not itself be Chinese.
        assert!(warning.is_ascii(), "{warning}");
    }

    #[test]
    fn a_loaded_font_has_nothing_to_warn_about() {
        let status = FontStatus::Loaded { path: PathBuf::from("/some/font.otf"), bytes: 42 };
        assert!(status.is_loaded());
        assert!(status.describe().contains("42 bytes"));
        assert_eq!(status.warning(), None);
    }

    #[test]
    fn without_the_font_the_panels_own_text_has_no_glyphs() {
        // The baseline half of the pair below: without it, "the font fixed it" would
        // be an assertion with nothing to compare against.
        let ctx = egui::Context::default();
        pass(&ctx);
        assert!(
            !covers_panel_text(&ctx),
            "egui's built-in set is Latin and emoji only, so a Chinese glyph here means this test is broken",
        );
    }

    /// The end-to-end check, and the reason this module is worth testing at all.
    ///
    /// Skipped — with a printed reason, because a silent skip reads like a pass — when
    /// the font has not been fetched (`npm run fonts`). Everything else in this file is
    /// path logic; this is the only test that proves a Chinese string reaches a glyph.
    #[test]
    fn a_bundled_font_makes_the_panels_own_text_drawable() {
        // The asset itself, not a staged copy: no layout puts it next to the test
        // binary, and this is the file the fetch script verifies.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts").join(FONT_FILE);
        if !path.is_file() {
            println!("skipping: {} is missing; run `npm run fonts`", path.display());
            return;
        }

        let ctx = egui::Context::default();
        let status = install_from(&ctx, &path);
        assert!(status.is_loaded(), "{}", status.describe());
        assert_eq!(status.warning(), None);

        pass(&ctx);

        assert!(
            covers_panel_text(&ctx),
            "the font was loaded from {path:?} but the panel's own text still has no glyphs",
        );
        // And the Latin the panel also draws must not have been displaced.
        assert!(ctx.fonts_mut(|fonts| fonts.has_glyphs(&egui::FontId::proportional(14.0), "quorfloat")));
    }
}
