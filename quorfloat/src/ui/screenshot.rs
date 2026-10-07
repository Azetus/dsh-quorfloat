//! One picture of the panel, for the people building it.
//!
//! This exists because the panel draws two things that cannot be checked by reading code —
//! its rounded corners and its own shadow — and because a window that is mostly transparent
//! cannot be checked by asking the platform for a screenshot either: `screencapture` needs a
//! screen-recording permission the development process does not have, while egui will hand
//! over its own framebuffer to anyone who asks.
//!
//! **The file is a PPM, and that is deliberate.** Writing a PNG needs an encoder, and an
//! encoder is a dependency the shipped panel would carry for a development aid. PPM is four
//! header lines and the raw pixels, every image tool on this machine reads it, and
//! `sips -s format png` turns it into something an eye can be shown.
//!
//! **The transparent parts are composited over mid grey.** A framebuffer has no desktop
//! behind it, so an unwritten pixel is not "nothing", it is a colour nobody would ever see.
//! Mid grey is the neutral stand-in the panel is judged against, and the compositing is
//! where a shadow's softness becomes visible instead of turning into a black fringe.

use std::io::Write;
use std::path::Path;

/// The colour transparent pixels are shown against.
///
/// Mid grey: dark enough that a light panel's shadow reads, light enough that a dark panel's
/// edge does, and never mistaken for part of the panel.
pub const BACKDROP: u8 = 0x80;

/// How long the file is allowed to be stale, while a dump is being kept up to date.
///
/// A second: fast enough that a person editing the panel sees their change by the time they look at the
/// file, slow enough that this stays a development aid rather than a per-frame encoding job.
pub const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether it is time to ask for another picture.
///
/// Pure, and separate from the asking, because this is the rule that was wrong: the first version asked
/// once and stopped, so the picture on disk was the first frame — of a window whose height is a result of
/// drawing, which means "the bottom of the panel was still missing" rather than "the change did not
/// apply". A rule with that failure mode should be checkable without a window.
///
/// @param visible - whether the panel is on screen; a hidden window has no framebuffer to hand over.
/// @param outstanding - whether a request is already in flight, which must not be repeated.
/// @param last - when the last picture was written, if any.
/// @returns whether to ask for one now.
#[must_use]
pub fn due(
    visible: bool,
    outstanding: bool,
    last: Option<std::time::Instant>,
) -> bool {
    if !visible || outstanding {
        return false;
    }
    match last {
        None => true,
        Some(last) => last.elapsed() >= REFRESH,
    }
}

/// How often a refresh is worth a line in the marker.
///
/// The marker is the only thing that can answer "was the dump still updating, or did it stop" after a
/// run, and one line at startup answers it wrongly. Every ten seconds is often enough to see the
/// difference and rare enough that the file stays readable.
pub const RECORD_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether this refresh is worth recording.
///
/// Pure, like [`due`], so the rule can be checked without a window.
///
/// @param last - when the last one was recorded, if any.
/// @param now - the current time.
/// @returns whether to write a line.
#[must_use]
pub fn worth_recording(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    match last {
        None => true,
        Some(last) => now.duration_since(last) >= RECORD_EVERY,
    }
}

/// Write the panel as a binary PPM, composited over [`BACKDROP`].
///
/// @param path - where to write.
/// @param image - the framebuffer egui handed over.
/// @returns what the filesystem said.
pub fn write_ppm(path: &Path, image: &eframe::egui::ColorImage) -> std::io::Result<()> {
    let [width, height] = image.size;
    let mut body = Vec::with_capacity(width * height * 3);
    for pixel in &image.pixels {
        body.extend_from_slice(&over_backdrop(*pixel));
    }

    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    let mut file = std::fs::File::create(path)?;
    write!(file, "P6\n{width} {height}\n255\n")?;
    file.write_all(&body)
}

/// One pixel, composited over the backdrop.
///
/// egui's colours are premultiplied, so "over" is a multiply and an add: the pixel as it is,
/// plus the backdrop scaled by how much of the pixel is *not* there. Getting this wrong is
/// how a transparent window turns into a black one in a screenshot — which is the very
/// mistake that made the picture worth taking.
///
/// @param pixel - the framebuffer pixel, premultiplied.
/// @returns its red, green and blue over the backdrop.
#[must_use]
pub fn over_backdrop(pixel: eframe::egui::Color32) -> [u8; 3] {
    let remaining = u32::from(255 - pixel.a());
    let channel = |value: u8| {
        // Rounded, not truncated: half of 0x80 is 0x40, and a division that answered 0x3f
        // would put a one-pixel-dark seam along every soft edge — which is exactly the kind
        // of thing this picture is taken to look for.
        let composited = u32::from(value) + (u32::from(BACKDROP) * remaining + 127) / 255;
        composited.min(255) as u8
    };
    [channel(pixel.r()), channel(pixel.g()), channel(pixel.b())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn the_picture_is_kept_up_to_date_rather_than_taken_once() {
        // The bug this exists for: the dump wrote the first frame and stopped. The window's height
        // follows its content, so that frame showed a panel still growing, and anything below the fold
        // was missing — which reads as "my change did not apply" and sent this project chasing a layout
        // problem that was not there. A second later, another picture.
        let now = std::time::Instant::now();
        assert!(due(true, false, None), "the first one is taken immediately");
        assert!(!due(true, true, None), "and not asked for twice while one is in flight");
        assert!(!due(false, false, None), "a hidden window has nothing to hand over");
        assert!(due(true, false, Some(now - REFRESH)), "once the file is a second old, again");
        assert!(due(true, false, Some(now - REFRESH * 2)), "and again");
        assert!(!due(true, false, Some(now)), "but not before it is stale");
    }

    #[test]
    fn the_record_of_the_dump_is_periodic_rather_than_one_shot() {
        // "The dump stopped updating" and "the dump is fine" have to be distinguishable from the marker
        // alone, and a single line written at startup makes them look identical.
        let now = std::time::Instant::now();
        assert!(worth_recording(None, now), "the first write is recorded");
        assert!(!worth_recording(Some(now), now), "and not every refresh after it");
        assert!(
            worth_recording(Some(now - RECORD_EVERY), now),
            "but a later one is, so the marker keeps saying the dump is alive",
        );
    }

    #[test]
    fn an_opaque_pixel_is_written_as_it_is() {
        assert_eq!(over_backdrop(Color32::from_rgb(0xfa, 0xfa, 0xf9)), [0xfa, 0xfa, 0xf9]);
        assert_eq!(over_backdrop(Color32::from_rgb(0x00, 0x00, 0x00)), [0, 0, 0]);
    }

    #[test]
    fn a_transparent_pixel_becomes_the_backdrop() {
        // Including a *black* transparent pixel: premultiplied black at zero alpha is
        // nothing at all, and a screenshot that showed it as black would be lying about what
        // the desktop behind the panel looks like.
        assert_eq!(over_backdrop(Color32::TRANSPARENT), [BACKDROP; 3]);
        assert_eq!(over_backdrop(Color32::from_rgba_premultiplied(0, 0, 0, 0)), [BACKDROP; 3]);
    }

    #[test]
    fn a_half_transparent_pixel_is_halfway_to_the_backdrop() {
        let half = over_backdrop(Color32::from_rgba_premultiplied(0, 0, 0, 128));
        assert_eq!(half, [BACKDROP / 2; 3], "half the pixel is nothing, so half the backdrop shows");
        // And a window cleared to nothing at all comes out as the backdrop rather than as
        // the near-black eframe clears with by default.
        let cleared = over_backdrop(Color32::from_rgba_unmultiplied(12, 12, 12, 180));
        assert!(cleared[0] > 12, "even a nearly-black backdrop-looking clear is composited: {cleared:?}");
    }

    #[test]
    fn the_file_is_a_ppm_with_the_size_in_its_header() {
        let path = std::env::temp_dir().join(format!("quorfloat-shot-{}.ppm", std::process::id()));
        let image = eframe::egui::ColorImage::new([2, 1], vec![Color32::WHITE, Color32::TRANSPARENT]);
        write_ppm(&path, &image).expect("write");
        let bytes = std::fs::read(&path).expect("read");
        assert!(bytes.starts_with(b"P6\n2 1\n255\n"), "header");
        assert_eq!(bytes.len(), 11 + 6, "header plus two pixels");
        assert_eq!(&bytes[11..14], &[0xff, 0xff, 0xff], "the opaque pixel");
        assert_eq!(&bytes[14..17], &[BACKDROP; 3], "and the transparent one");
        let _ = std::fs::remove_file(&path);
    }
}
