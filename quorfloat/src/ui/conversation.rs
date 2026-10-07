//! The conversation: the lines, and who they are from.

use eframe::egui;

use crate::app::PanelState;
use crate::ui::theme as theme;
use crate::ui::theme::CONVERSATION_MIN_HEIGHT;

use super::{icons, wrapped};
use crate::app::session::transcript::{Block, Entry};

/// Draw the conversation.
///
/// Sticks to the bottom, which is the whole of the "follow the answer as it is written"
/// behaviour: egui only keeps a scroll area pinned while it is already at the bottom, so
/// a user who has scrolled up to read something is not yanked back by the next token.
///
/// @param ui - where to draw.
/// @param state - the conversation, plus the message still being generated.
///
/// @param height - the space left after the header, the cards and the composer's strip.
///   Passed in rather than measured so that the composer, which is drawn afterwards, has
///   already claimed its share.
///
/// @returns how tall the conversation's content is, which is what the window's height is
///   grown from. The height handed in is the space it may *use*; this is the space it
///   *wants*, and the two differ exactly when the conversation is short.
pub(super) fn conversation(
    ui: &mut egui::Ui,
    state: &PanelState,
    height: f32,
    markdown: &mut egui_commonmark::CommonMarkCache,
) -> f32 {
    // The floor is a floor for a *window*, not a claim on space that is not there: past it
    // the conversation is clipped at the bottom — above the composer, which is the right
    // thing to lose.
    let height = height.max(CONVERSATION_MIN_HEIGHT.min(height.max(0.0))).max(0.0);
    let output = egui::ScrollArea::vertical()
        // Shrink to the content vertically, fill horizontally. This is what keeps the drawn
        // panel the same height as the height the window was asked for: with
        // `auto_shrink([false, false])` the area *fills* whatever it is offered, so a short
        // conversation drew 57px of empty space where the layout had counted 17 — and the
        // panel came out 40px taller than its window, with the footer clipped off the bottom
        // (see `docs/progress.md` §30).
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if state.entries.is_empty() && state.live.is_none() {
                ui.label(theme::meta(ui.ctx(), "尚无对话内容"));
                return;
            }
            // Turns are drawn as turns: the design separates them with a hairline, 20px above
            // and 18px below, which is what makes a long conversation scannable — the eye finds
            // the questions without reading the answers (`.q-turn + .q-turn`).
            let mut first_turn = true;
            for entry in state.entries.iter() {
                if matches!(entry, Entry::User { .. }) {
                    if !first_turn {
                        ui.add_space(theme::TURN_GAP_ABOVE);
                        separator(ui);
                        ui.add_space(theme::TURN_GAP_BELOW);
                    }
                    first_turn = false;
                }
                entry_ui(ui, entry, markdown);
            }
            if let Some(live) = &state.live {
                entry_ui(ui, live, markdown);
            }
        });
    output.content_size.y
}

/// The line under an answer: what happened to it, and the copy control.
///
/// @param ui - where to draw.
/// @param blocks - the answer's blocks, whose text is what a copy takes.
/// @param streaming - whether the answer is still arriving.
/// @param markdown - the viewer's cache, unused here but kept for symmetry with the caller.
fn answer_bar(
    ui: &mut egui::Ui,
    blocks: &[Block],
    streaming: bool,
    _markdown: &mut egui_commonmark::CommonMarkCache,
) {
    ui.add_space(theme::ANSWER_BAR_GAP);
    ui.horizontal(|ui| {
        // The status is what tells a finished answer from a stalled one, and the design words it
        // as a fact rather than a spinner.
        let status = if streaming { "正在生成" } else { "回答完成" };
        ui.label(theme::meta(ui.ctx(), status));
        // Nothing to copy while there is nothing to copy: the design hides the button rather
        // than offering a control that would put an empty string on the clipboard.
        let source = answer_source(blocks);
        if source.is_empty() {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let id = copy_id(&source);
            let copied = ui.memory(|memory| memory.data.get_temp::<bool>(id).unwrap_or(false));
            let label = if copied { "已复制" } else { "复制回答" };
            let button = ui.add(
                egui::Button::new(
                    egui::RichText::new(label).font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META)).color(theme::muted()),
                )
                .frame(false)
                .min_size(egui::vec2(0.0, 0.0)),
            );
            if button.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if button.clicked() {
                // The *source*, not the rendered text: what a user pastes into an editor should
                // be the Markdown the model wrote, not the words as this panel happens to lay
                // them out. (The mockup copies rendered text because its answers are plain.)
                ui.ctx().copy_text(source.clone());
                ui.memory_mut(|memory| memory.data.insert_temp(id, true));
            }
        });
    });
}

/// The text a copy of one answer takes: its prose, with the Markdown intact.
///
/// @param blocks - the answer's blocks.
/// @returns the source text, or an empty string when the answer has no prose yet.
#[must_use]
pub(super) fn answer_source(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The identity a "copied" state is remembered under.
///
/// @param source - the answer's text.
/// @returns the egui id for its state.
#[must_use]
pub(super) fn copy_id(source: &str) -> egui::Id {
    reasoning_id(source)
}

/// The who-said-it line, for the entries the design has no shape for.
fn speaker_line(ui: &mut egui::Ui, entry: &crate::app::session::transcript::Entry) {
    ui.label(egui::RichText::new(speaker(entry)).size(theme::TEXT_META).color(theme::muted()));
    ui.add_space(2.0);
}

/// A hairline across the thread, the design's separator between turns.
fn separator(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, theme::BORDER), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0, theme::line());
}

/// The reasoning blocks of one assistant message, joined.
///
/// One string rather than a list, because the fold is one fold: the working-out of a single
/// answer reads as one piece, and the harness may split it across blocks.
///
/// @param blocks - the message's blocks.
/// @returns the reasoning, or an empty string when there is none.
#[must_use]
pub(super) fn reasoning_text(
    blocks: &[crate::app::session::transcript::Block],
) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Reasoning(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The identity a fold's open/closed state is remembered under.
///
/// Keyed by the text rather than by position: the transcript drops its oldest entries as it
/// grows, and a fold that jumped to another answer when the buffer trimmed would be worse than
/// one that forgot.
///
/// @param reasoning - the reasoning text.
/// @returns the egui id for its fold.
#[must_use]
pub(super) fn reasoning_id(reasoning: &str) -> egui::Id {
    egui::Id::new(("quorfloat-reasoning", text_hash(reasoning)))
}

/// Give the layout somewhere to break inside long unbroken runs.
///
/// egui wraps at word boundaries and, deliberately, never inside a word unless it is truncating
/// (`egui::Label` sets `break_anywhere` only for `TextWrapMode::Truncate`). Chinese text is
/// unaffected — it breaks between characters — but a code block full of JSON is one long word per
/// line, and it is laid out at its full width however narrow the panel is.
///
/// Inserting U+200B (zero-width space) every [`SOFT_WRAP_EVERY`] characters of a run that has no
/// whitespace gives the wrapper a candidate. It draws nothing, and it is *not* what a copy takes:
/// the copy control uses the untouched source, so a pasted answer is byte-for-byte the model's.
///
/// @param text - the answer's Markdown source.
/// @returns the same text, with break opportunities added to its long runs.
#[must_use]
pub(crate) fn soft_wrap_for_display(text: &str) -> String {
    /// How far a run may go before it is given a break opportunity. Wide enough that ordinary
    /// words and short identifiers are never touched.
    const SOFT_WRAP_EVERY: usize = 24;
    let mut out = String::with_capacity(text.len() + text.len() / SOFT_WRAP_EVERY);
    let mut run = 0;
    for character in text.chars() {
        // Whitespace is already a break opportunity, and a newline is a hard one.
        if character.is_whitespace() {
            run = 0;
        } else {
            run += 1;
            if run > SOFT_WRAP_EVERY {
                out.push('\u{200b}');
                run = 1;
            }
        }
        out.push(character);
    }
    out
}

/// The id scope one rendered answer's widgets live under.
///
/// The Markdown viewer ids its tables from the `Ui` they are drawn into plus a counter that
/// **restarts with every call** (`ui.id().with("_table").with(curr_table)`, `pulldown.rs`), so two
/// answers sharing a `Ui` id collide on their first table — which egui reports on screen as
/// "Second use of Grid ID …". An id keyed by the answer's own text gives each one its own
/// namespace, however many answers the frame happens to draw (see `docs/progress.md` §39).
///
/// @param text - the answer's Markdown source.
/// @returns the egui id to scope it with.
#[must_use]
pub(super) fn answer_id(text: &str) -> egui::Id {
    egui::Id::new(("quorfloat-answer", text_hash(text)))
}

/// FNV-1a, which is enough for these: the ids only have to separate the handful of answers on
/// screen, and a collision would fold two of them together rather than lose anything.
fn text_hash(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A model's answer, in the design's own proportion.
///
/// The line height is the point: an answer is prose to be read rather than scanned, and
/// `line-height:1.85` is what the design uses to keep a long one readable. It is set on the
/// text rather than through the style so that a wrapped paragraph and the reasoning block
/// beside it do not drift apart.
///
/// @param ctx - the context, for the font the weights are cut from.
/// @param text - the answer so far.
/// @returns the text to lay out.
fn answer(ctx: &egui::Context, text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .font(theme::font(ctx, theme::Weight::Regular, theme::TEXT_BODY))
        .line_height(Some(theme::LINE_ANSWER))
        .color(theme::text())
}

/// Draw one line of the conversation.
///
/// @param ui - where to draw.
/// @param entry - the line.
pub(super) fn entry_ui(
    ui: &mut egui::Ui,
    entry: &crate::app::session::transcript::Entry,
    markdown: &mut egui_commonmark::CommonMarkCache,
) {
    use crate::app::session::transcript::{Block, Entry};

    ui.add_space(6.0);
    match entry {
        // The design's own shape for a turn: the question is one muted line, prefixed with who
        // asked it — `你 · …` — and the answer is not introduced at all. A label above every
        // answer is a heading nobody reads, and the answer bar below already says what it is.
        Entry::User { text } => {
            wrapped(
                ui,
                egui::RichText::new(format!("你 · {text}"))
                    .font(theme::font(ui.ctx(), theme::Weight::Regular, theme::TEXT_META))
                    .color(theme::muted()),
            );
            ui.add_space(theme::QUESTION_GAP);
        }
        Entry::Assistant { blocks, streaming } => {
            // The model's working-out, folded away. It is kept rather than dropped — it is what
            // the model is doing, and a panel that shows only conclusions makes a slow answer
            // look stuck — but it is not what the user came to read, so it is one click away
            // instead of always on screen. The design has no opinion here; the request for it
            // came from using the panel (see `docs/progress.md` §35).
            let reasoning = reasoning_text(blocks);
            if !reasoning.is_empty() {
                let id = reasoning_id(&reasoning);
                let open = ui.memory(|memory| memory.data.get_temp::<bool>(id).unwrap_or(false));
                let label = if *streaming && !open { "思考中…" } else { "思考" };
                let row = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let mark = if open { icons::Icon::CaretDown } else { icons::Icon::CaretRight };
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(theme::ICON_CHEVRON, theme::ICON_CHEVRON),
                            egui::Sense::hover(),
                        );
                        icons::paint(ui, rect.center(), mark, theme::ICON_CHEVRON, theme::muted());
                        ui.label(
                            egui::RichText::new(label).size(theme::TEXT_SMALL).color(theme::muted()),
                        );
                    })
                    .response
                    .interact(egui::Sense::click());
                if row.clicked() {
                    ui.memory_mut(|memory| memory.data.insert_temp(id, !open));
                }
                if row.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if open {
                    wrapped(
                        ui,
                        egui::RichText::new(&reasoning)
                            .size(theme::TEXT_META)
                            .color(theme::muted())
                            .italics(),
                    );
                }
            }
            for block in blocks {
                match block {
                    // Markdown, once the answer has stopped growing. While it streams the text
                    // is drawn as it is: half a fence or an unclosed `**` renders as literal
                    // punctuation for a moment and then reflows, and watching an answer
                    // rearrange itself is worse than watching it arrive plainly.
                    Block::Text(text) => {
                        if *streaming {
                            wrapped(ui, answer(ui.ctx(), text));
                        } else {
                            // An id scope per answer, because `ui.scope` is *not* one: it restores
                            // the style but shares the id namespace, and the Markdown viewer draws
                            // tables with an auto-id `Grid`. Two of them in one frame collide, and
                            // egui says so on screen — "Second use of Grid ID 2A87 … Sometimes the
                            // solution is to use ui.push_id", which is exactly this (see
                            // `docs/progress.md` §39).
                            ui.push_id(answer_id(text), |ui| {
                                let style = theme::markdown_style(ui.style(), theme::palette());
                                ui.style_mut().clone_from(&style);
                                let available = ui.available_width().max(1.0);
                                ui.set_max_width(available);
                                // Tables are drawn by this panel (`super::table`), because the
                                // viewer draws them as an `egui::Grid` — measured from its content,
                                // with no scroll of its own — and one wide cell painted past the
                                // panel's edge. Everything else goes to the viewer as one piece, so
                                // prose is laid out exactly as it was before (see §40).
                                // The viewer draws the answer, with the tables bounded: the
                                // parser tells us where they are, and nothing else about the
                                // Markdown is decided here (see `super::table`).
                                super::table::draw(ui, markdown, text, available);
                            });
                        }
                    }
                    // Drawn above, folded; never twice.
                    Block::Reasoning(_) => {}
                    Block::Call { name, arguments } => {
                        wrapped(ui, egui::RichText::new(format!("$ {name} {arguments}")).size(theme::TEXT_META).color(theme::muted()).monospace());
                    }
                }
                ui.add_space(2.0);
            }
            // The bar replaces the bare "生成中…" this used to be: the design puts the turn's
            // status and the copy control on one line under the answer, and the status is what
            // says whether the answer is finished (`.q-answerbar`).
            answer_bar(ui, blocks, *streaming, markdown);
        }
        Entry::Tool { text, is_error, .. } => {
            speaker_line(ui, entry);
            let colour = if *is_error { theme::bad_text() } else { theme::muted() };
            wrapped(ui, egui::RichText::new(text).size(theme::TEXT_META).color(colour).monospace());
        }
        // The panel's own additions, and the only entries that keep a who-said-it line: the
        // design has no shape for a tool result or an unrecognised event, so they introduce
        // themselves.
        Entry::System { text } => {
            speaker_line(ui, entry);
            wrapped(ui, theme::meta(ui.ctx(), text));
        }
        Entry::Notice { text, .. } => {
            speaker_line(ui, entry);
            wrapped(ui, theme::meta(ui.ctx(), text));
        }
    }
}

/// Who a line is from, as the panel labels it.
///
/// @param entry - the line.
/// @returns the label above it.
#[must_use]
pub(crate) fn speaker(entry: &crate::app::session::transcript::Entry) -> String {
    use crate::app::session::transcript::Entry;
    match entry {
        Entry::User { .. } => "你".to_owned(),
        Entry::Assistant { .. } => "quorfloat".to_owned(),
        Entry::Tool { name, .. } => {
            name.as_ref().map_or_else(|| "工具".to_owned(), |name| format!("工具 · {name}"))
        }
        Entry::System { .. } => "系统".to_owned(),
        // The kind is in the label rather than only in the text: an unrecognised event
        // is exactly the case where the reader needs to know what it was called.
        Entry::Notice { kind, .. } => format!("事件 · {kind}"),
    }
}
