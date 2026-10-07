//! Fonts: making the panel able to draw more than Latin.
//!
//! egui's built-in set is Latin (Ubuntu-Light) plus a small emoji subset, so every
//! Chinese string this panel writes would otherwise render as the box epaint
//! substitutes for a character no font in the family can draw.
//!
//! ## Why a bundled font instead of the system's
//!
//! Reading a system font is cheaper to *write* and worse to *ship*, and this was
//! measured on a real machine rather than argued (`docs/progress.md` §22):
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

/// Name the CJK font is registered under inside egui.
const FAMILY: &str = "noto-sans-sc";

/// The family the medium (500) cut of the bundled variable font is registered under.
pub const WEIGHT_MEDIUM: &str = "noto-sans-sc-medium";

/// The family the semibold (600) cut is registered under.
pub const WEIGHT_SEMIBOLD: &str = "noto-sans-sc-semibold";

/// The context-data key under which "the weighted cuts are installed" is recorded.
///
/// **Per context, not per process.** epaint panics on a font family that is bound to no
/// fonts, so the answer to "may I ask for the medium cut?" has to come from the context
/// that will draw the text. A process-wide flag looked simpler and was wrong: tests run in
/// one process with a context each, so a test that installed the font made every *other*
/// test ask for families its own context had never heard of — which is a panic, and is
/// exactly how this was found.
const WEIGHTED_KEY: &str = "quorfloat.fonts.weighted";

/// Whether the weighted cuts of the bundled font are installed in this context.
///
/// @param ctx - the render context that would draw the text.
/// @returns `true` when asking for [`WEIGHT_MEDIUM`] or [`WEIGHT_SEMIBOLD`] is safe.
#[must_use]
pub fn has_weighted_families(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new(WEIGHTED_KEY)).unwrap_or(false))
}

/// The family the icon font is registered under, and the only way to draw with it.
///
/// The icon font is compiled into this binary (`egui-phosphor` carries the bytes), so
/// unlike the CJK font it cannot be missing — and unlike the CJK font it is **never** a
/// fallback: an icon font in the proportional chain means body text can render a padlock
/// where a character was meant to be.
pub const ICON_FAMILY: &str = "icons";

/// The panel's own text, used to check that the font actually arrived.
///
/// Taken from the UI rather than invented, so the check fails when the *panel* cannot
/// be read — which is the failure a user would actually notice.
const PANEL_SAMPLE: &str = "允许一次 拒绝";

/// What happened when the fonts were installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontStatus {
    /// The bundled CJK font is in use. The icons may or may not be, and that is reported
    /// separately because it is a different kind of loss: without the CJK font Chinese is
    /// unreadable, without the icons the controls are unlabelled.
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
                format!("fonts loaded {FAMILY} from {} ({bytes} bytes), icons from phosphor", path.display())
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
    locate_named(exe_dir, FONT_PATH_ENV, FONT_FILE, "fonts")
}

/// The one lookup rule, used for both fonts.
///
/// @param exe_dir - directory holding this executable.
/// @param variable - the environment variable that overrides everything.
/// @param file - the file name to look for.
/// @param directory - the directory name inside the package and beside the binary.
/// @returns the first candidate that exists, and every path that was tried.
#[must_use]
fn locate_named(exe_dir: Option<&Path>, variable: &str, file: &str, directory: &str) -> (Option<PathBuf>, Vec<PathBuf>) {
    let override_path = std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    if let Some(path) = override_path {
        push(path);
    }
    if let Some(directory_of_exe) = exe_dir {
        let parent = directory_of_exe.parent();
        let mut layouts: Vec<PathBuf> = Vec::new();
        // The platform package: `bin/dsh-quorfloat` with `fonts/` and `icons/` beside `bin/`.
        if let Some(root) = parent {
            layouts.push(root.join(directory));
        }
        // A flat layout, which is what the development harness stages.
        layouts.push(directory_of_exe.join(directory));
        for layout in layouts {
            push(layout.join(file));
        }
    }
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
    ensure_icons(ctx);
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
    // First, not last: the design's body text is a normal-weight humanist sans, and the
    // built-in Latin face is a *light* one — leaving it ahead of the bundled font would
    // make every Latin sentence thinner than the design asks for. The built-in faces stay
    // behind it as fallbacks for anything this font does not cover.
    ctx.add_font(egui::epaint::text::FontInsert {
        name: FAMILY.to_owned(),
        data: egui::FontData::from_static(shared),
        families: vec![
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Highest,
            },
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::epaint::text::FontPriority::Highest,
            },
        ],
    });
    // The design's 500 and 600 weights, cut from the same variable font. One copy of the
    // bytes, three families: the coordinates are applied when a glyph is rasterised, so
    // this costs three atlas entries and no extra memory for the font itself.
    for (family, weight) in [(WEIGHT_MEDIUM, 500.0), (WEIGHT_SEMIBOLD, 600.0)] {
        ctx.add_font(egui::epaint::text::FontInsert {
            name: family.to_owned(),
            data: egui::FontData::from_static(shared).tweak(egui::epaint::text::FontTweak {
                coords: egui::epaint::text::VariationCoords::new([(b"wght", weight)]),
                ..Default::default()
            }),
            families: vec![egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Name(family.into()),
                priority: egui::epaint::text::FontPriority::Highest,
            }],
        });
    }
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(WEIGHTED_KEY), true));
    FontStatus::Loaded { path: path.to_path_buf(), bytes: size }
}

/// The context-data key recording that the icon family is registered here.
const ICONS_KEY: &str = "quorfloat.fonts.icons";

/// Whether the icon family can be drawn with *in this pass*.
///
/// Asked by [`crate::ui::icons::glyph`] before it names the family, because egui applies a
/// pending font at the **end** of a pass (`Context::add_font` queues it, and the queue is
/// drained after the frame is built). A context that asks for the icons and then draws in
/// the same pass would name a family that is not bound to any fonts yet — which epaint
/// answers with a panic rather than with a blank space. It was found exactly that way, by
/// tests that draw a panel into a context `main` never installed into.
///
/// The pass number is the whole mechanism, and it is egui's own public one: asked-for in
/// pass N means usable from pass N+1.
///
/// @param ctx - the render context.
/// @returns whether drawing with [`ICON_FAMILY`] is safe right now.
#[must_use]
pub fn icons_ready(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<u64>(egui::Id::new(ICONS_KEY)))
        .is_some_and(|asked_in_pass| ctx.cumulative_pass_nr() > asked_in_pass)
}

/// Register the icon family once per context, whoever gets there first.
///
/// Unlike the CJK font this reads no file — the bytes are compiled in — so it is cheap
/// enough to guarantee from the drawing path rather than from a startup step someone has
/// to remember to call.
///
/// @param ctx - the render context.
pub fn ensure_icons(ctx: &egui::Context) {
    if ctx.data(|data| data.get_temp::<u64>(egui::Id::new(ICONS_KEY)).is_some()) {
        return;
    }
    install_icons(ctx);
    let asked_in_pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(ICONS_KEY), asked_in_pass));
}

/// Register the icon font under its own family.
///
/// **Not** a fallback: it is deliberately absent from the proportional and monospace
/// chains, so body text can never render a padlock where a character was meant. Drawing an
/// icon means asking for [`ICON_FAMILY`] by name — which is what `ui/icons.rs` does.
///
/// The bytes are `'static` because they come from the icon crate rather than from a file,
/// so there is nothing to read, nothing to leak, and no way for this to fail.
///
/// @param ctx - the render context.
pub fn install_icons(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert {
        name: ICON_FAMILY.to_owned(),
        data: egui::FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()),
        families: vec![egui::epaint::text::InsertFontFamily {
            family: egui::FontFamily::Name(ICON_FAMILY.into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    });
}

/// The font an icon is drawn with.
///
/// @param size - the icon's box, in points.
/// @returns the font id to lay an icon glyph out with.
#[must_use]
pub fn icon_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(ICON_FAMILY.into()))
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

    /// The icon font, and the two properties that matter about it.
    ///
    /// This one never skips: the icon bytes are compiled in, so "no icons" is not a state
    /// this build can be in.
    #[test]
    fn the_icon_font_draws_icons_and_nothing_else() {
        let ctx = egui::Context::default();
        install_icons(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();

        // Every icon in the panel's vocabulary is drawable through its own family...
        for icon in crate::ui::icons::ALL {
            let chars = icon.chars();
            assert!(
                ctx.fonts_mut(|fonts| fonts.has_glyph(&icon_font(16.0), chars.chars().next().unwrap())),
                "the icon family has {}",
                icon.name(),
            );
        }
        // ...and none of them is reachable through the text families, which is the whole
        // reason the icon font is registered by name: a body paragraph must never be able to
        // render a padlock.
        let folder = crate::ui::icons::Icon::Folder;
        assert!(
            !ctx.fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::proportional(16.0), folder.chars().chars().next().unwrap())),
            "and the text family does not, so an icon cannot leak into a sentence",
        );
    }

    /// The weighted cuts are a fact about a *context*, not about the process.
    ///
    /// This is how the parallel-test panic was found: a process-wide flag was set by the
    /// test that installed the font, and every other test then asked for families its own
    /// context had never heard of — which epaint answers with a panic.
    #[test]
    fn installing_the_font_marks_only_the_context_it_was_installed_into() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts").join(FONT_FILE);
        if !path.is_file() {
            println!("skipping: {} is missing; run `npm run fonts`", path.display());
            return;
        }
        let installed = egui::Context::default();
        let untouched = egui::Context::default();
        assert!(!has_weighted_families(&untouched), "nothing is installed to begin with");
        install_from(&installed, &path);
        assert!(has_weighted_families(&installed), "and the context that installed it knows");
        assert!(
            !has_weighted_families(&untouched),
            "while a context that did not is unaffected — which is what keeps a missing font \
             from becoming a panic instead of a warning",
        );
    }

    /// The timing the guard in [`crate::ui::icons::glyph`] exists for.
    ///
    /// If this ever becomes "ready immediately", the guard is unnecessary — and if it
    /// becomes "ready one pass later than this says", the guard is wrong and the panel
    /// panics on its first frame in any context. Either way this test is where that is
    /// noticed.
    #[test]
    fn the_icon_family_is_usable_from_the_pass_after_it_was_asked_for() {
        let ctx = egui::Context::default();
        assert!(!icons_ready(&ctx), "nothing has been asked for yet");

        ensure_icons(&ctx);
        assert!(!icons_ready(&ctx), "asked for during this pass, applied at the end of it");

        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert!(icons_ready(&ctx), "and from the next pass it can be drawn with");

        // Asking again is idempotent, and does not push the answer into the future.
        ensure_icons(&ctx);
        assert!(icons_ready(&ctx), "a second ask does not un-ready it");
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
