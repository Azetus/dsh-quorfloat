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
        // The footer has already reserved its space; egui's default 64px minimum
        // must not take that space back while the native window is still growing.
        .min_scrolled_height(0.0)
        .max_height(height)
        // **A gutter for the scroll bar**, which floats over the content by default: without it the bar
        // lies on the rightmost characters of a long line while the user scrolls (the report), and the
        // copy control under an answer is clipped by it too. The width is egui's own
        // (`theme::scroll_gutter`).
        .content_margin(egui::Margin { right: theme::scroll_gutter(ui.style()), ..egui::Margin::ZERO })
        .show(ui, |ui| {
            if state.entries.is_empty() && state.live.is_none() {
                ui.label(theme::meta(ui.ctx(), "尚无对话内容"));
                return;
            }
            // **Every turn is drawn as two parts: one disclosure, then the answer.**
            //
            // **The rule is this project's own, and deliberately simple**: everything the model
            // produced that is not the turn's final answer goes behind `已完成` — its reasoning, its
            // tool calls and their results, and any prose it wrote along the way. What stays out is
            // the final answer (what the user came to read) and the question (the user's own words).
            //
            // We no longer chase the upstream Harness's presentation (`turn-process`, with its
            // per-step `latestAnswer`, `groupPart` splitting of one message, forced-open aborted
            // turns, …): it is a different client with different furniture around the same events.
            // Where a rule here still agrees with upstream, that is a coincidence of both being
            // simple — not a compatibility goal. See `docs/dsh-quorfloat.md` §11.
            //
            // Turns are separated by a hairline, 20px above and 18px below, which is what makes a
            // long conversation scannable (`.q-turn + .q-turn`).
            let total = state.entries.len();
            let mut first_turn = true;
            let mut index = 0;
            // A `loop` rather than `while index < total`, because an empty transcript still has one
            // thing worth drawing: the answer that is being written before any line of it has landed.
            loop {
                let turn_end = next_turn_start(&state.entries, index);
                let is_last = turn_end == total;
                if !first_turn {
                    ui.add_space(theme::TURN_GAP_ABOVE);
                    separator(ui);
                    ui.add_space(theme::TURN_GAP_BELOW);
                }
                first_turn = false;
                // The answer being written belongs to the last turn, and is drawn as part of it: its
                // reasoning joins that turn's working, its text is that turn's answer.
                let live = if is_last { state.live.as_ref() } else { None };
                // **A turn still being worked on says so, and stays folded.** While the model works, its
                // disclosure carries "正在工作" and a turning mark — it is *not* opened for the reader:
                // what is inside is exactly what is about to be folded anyway, so streaming it in full
                // and then collapsing it is the panel jumping for no reason the reader can see (the
                // report). (`live` alone counts as working: a turn whose stream is still arriving is
                // being worked on even if its `turn/end` has already been folded in.)
                let working = is_last && (state.turn_active || live.is_some());
                turn_ui(ui, index, &state.entries[index..turn_end], live, working, markdown);
                index = turn_end;
                if index >= total {
                    break;
                }
            }
        });
    output.content_size.y
}

/// Where the next turn starts, so one turn's lines can be drawn together.
///
/// A turn begins at a user line. Anything before the first one — a notice, a system line — is its own
/// leading group, which keeps the caller free of a special case.
///
/// @param entries - the transcript's lines.
/// @param from - the index this turn starts at.
/// @returns the exclusive end index of the turn starting at `from`.
fn next_turn_start(entries: &[Entry], from: usize) -> usize {
    (from + 1..entries.len())
        .find(|index| matches!(entries[*index], Entry::User { .. }))
        .unwrap_or(entries.len())
}

/// Which line of a turn is its final answer, if it has one.
///
/// **The final answer is the turn's last assistant line that has reply text and no tool call.** A
/// message that ends in a tool call is the model still working, however much prose precedes the call:
/// it has not answered anything yet, and everything it wrote there is part of the working.
///
/// @param turn - the turn's lines.
/// @returns the answer's index within `turn`, or `None` while the turn has produced no answer.
fn answer_index(turn: &[Entry]) -> Option<usize> {
    turn.iter()
        .enumerate()
        .rev()
        .find(|(_, entry)| match entry {
            Entry::Assistant { blocks, .. } => {
                !blocks.iter().any(|block| matches!(block, Block::Call { .. }))
                    && blocks.iter().any(|block| match block {
                        Block::Text(text) => !text.trim().is_empty(),
                        _ => false,
                    })
            }
            _ => false,
        })
        .map(|(index, _)| index)
}

/// One turn: its working behind a disclosure, then its answer.
///
/// @param ui - where to draw.
/// @param at - where this turn starts among the transcript's lines. Part of the disclosure's identity
///   (see [`process_id`]) and nothing else.
/// @param turn - the turn's lines.
/// @param live - the answer still being written, when this is the turn writing it.
/// @param working - whether the turn is still being worked on. It labels the disclosure and turns its
///   mark; it does **not** open it (see [`process_fold`]).
/// @param markdown - the viewer's cache.
fn turn_ui(
    ui: &mut egui::Ui,
    at: usize,
    turn: &[Entry],
    live: Option<&Entry>,
    working: bool,
    markdown: &mut egui_commonmark::CommonMarkCache,
) {
    let split = split_turn(turn);
    if let Some(question) = split.question {
        entry_ui(ui, question, markdown);
    }
    // The answer in flight is a line of the working as far as its thinking goes: it has no line of
    // the turn yet, and while the model thinks it is the only thing happening.
    let mut held = split.held;
    if let Some(thinking) = live.filter(|entry| has_reasoning(entry)) {
        held.push(Held::Reasoning(thinking));
    }
    // **One disclosure, and only when there is something behind it.** A fold over nothing is a
    // control that does nothing; a second fold — the answer folding its own reasoning — is the same
    // fact asked twice, and closed, so opening the first would appear to reveal nothing.
    if !held.is_empty() {
        // The disclosure's identity: what the turn *is*, not what it currently holds — the held rows
        // change every frame while the model works (see [`process_id`]).
        let id = process_id(at, split.question, &held);
        process_fold(ui, id, &held, working, markdown);
    }
    for entry in &split.answer {
        entry_ui(ui, entry, markdown);
    }
    // Below the disclosure, where an answer belongs. The reasoning it may carry is inside it.
    if let Some(live) = live {
        entry_ui(ui, live, markdown);
    }
}

/// One thing a turn's disclosure holds.
///
/// The two cases are two answers to "is this message the final answer?": usually not (an earlier step,
/// a tool result, a narrated line — held whole), but the answering message itself can carry reasoning,
/// and **only its reasoning is not the answer.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held<'a> {
    /// A line of the working, drawn whole: its reasoning as text, then the calls it made and the
    /// prose it wrote around them.
    Working(&'a Entry),
    /// The reasoning of a line whose reply is drawn elsewhere — the answer's, or the one still being
    /// streamed. **Only the reasoning**: holding the reply as well would put it on screen twice.
    Reasoning(&'a Entry),
}

/// One turn, cut into the four parts the drawer needs.
///
/// A pure description of the split, so the rule can be tested without a frame: what is *hidden* by
/// the fold is the thing worth asserting, and a rendering test cannot see it (a fresh egui context
/// per call means a closed fold cannot be reopened to check what was behind it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnSplit<'a> {
    /// The user's question — the turn's first line, always drawn and never folded: it is the
    /// signpost that makes a long conversation scannable.
    question: Option<&'a Entry>,
    /// Everything the model produced except its final answer, in the order it happened. Empty when
    /// the turn produced nothing but its answer.
    held: Vec<Held<'a>>,
    /// The answer — everything from the answer line on.
    answer: Vec<&'a Entry>,
}

/// Cut one turn into its parts.
///
/// **Question, one disclosure, answer.** The question is never inside the fold — it is what the user
/// wrote, and the signpost that makes a long conversation scannable. The final answer stays outside it
/// too: it is the point of the turn. **Everything else the model produced is held** — reasoning, tool
/// calls, tool results, and the prose it narrated between them.
///
/// @param turn - the turn's lines.
/// @returns the parts, with `held` empty when the turn produced nothing but its answer.
fn split_turn(turn: &[Entry]) -> TurnSplit<'_> {
    let question = match turn.first() {
        Some(entry @ Entry::User { .. }) => Some(entry),
        _ => None,
    };
    let from = if question.is_some() { 1 } else { 0 };
    let answer_at = answer_index(turn);
    let answer_from = answer_at.unwrap_or(turn.len());
    // **Everything between the question and the answer is held.** There is no third case to weigh up:
    // a group runs from its question to the next one (`next_turn_start`), so the only lines it can
    // contain are the model's own — its reasoning, its tool calls, their results, its narration. The
    // one line that is drawn outside is the answer, and that is decided by `answer_index`.
    let mut held: Vec<Held<'_>> = turn[from..answer_from].iter().map(Held::Working).collect();
    // The answering message's own reasoning is part of the working, and it goes last: it is the last
    // thing the model thought before it answered. Holding the line *whole* would put the answer inside
    // the fold as well, and folding the answer's reasoning separately — which is what this panel used
    // to do — put a second disclosure on screen (`已完成` above `思考`), closed, so opening the first
    // revealed nothing about the thinking it exists to show.
    if let Some(at) = answer_at {
        if has_reasoning(&turn[at]) {
            held.push(Held::Reasoning(&turn[at]));
        }
    }
    TurnSplit { question, held, answer: turn[answer_from..].iter().collect() }
}

/// Whether a line carries reasoning of its own.
///
/// Blank blocks do not count: a disclosure opened onto an empty thought is a control that does
/// nothing.
///
/// @param entry - the line.
/// @returns whether there is reasoning to hold.
fn has_reasoning(entry: &Entry) -> bool {
    match entry {
        Entry::Assistant { blocks, .. } => blocks
            .iter()
            .any(|block| matches!(block, Block::Reasoning(text) if !text.trim().is_empty())),
        _ => false,
    }
}

/// The folded working: a header that says what the turn is doing, and the rows behind it.
///
/// **The program never opens it.** A turn's working is folded from its first frame — while the model
/// thinks as much as after it answers — because the alternative is what the reader reported: the working
/// streams in full, visibly, and then collapses the moment the answer lands, so the panel jumps for no
/// reason they can see. The reader may open it whenever they like, mid-stream included, and what they
/// open stays open (which is why the identity comes from [`process_id`] rather than from the rows).
///
/// @param ui - where to draw.
/// @param id - the disclosure's identity, from [`process_id`].
/// @param held - the rows the disclosure holds.
/// @param working - whether the turn is still being worked on.
/// @param markdown - the viewer's cache.
fn process_fold(
    ui: &mut egui::Ui,
    id: egui::Id,
    held: &[Held<'_>],
    working: bool,
    markdown: &mut egui_commonmark::CommonMarkCache,
) {
    // **Closed unless the reader opened it** — the default is not a phase of the turn, it is the state
    // of the control.
    let open = ui.memory(|memory| memory.data.get_temp::<bool>(id)).unwrap_or(false);
    let content = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(theme::ICON_CHEVRON, theme::ICON_CHEVRON),
                egui::Sense::hover(),
            );
            // **The caret keeps its place in both states** — it is the row's affordance, and the row is
            // clickable while the model works too. The turning mark is an *addition after the words*
            // (`▸ 正在工作 ◌`), not a swap for the caret: a different glyph in the caret's box put a
            // mark with different metrics where the caret had been, which reads as misaligned, and the
            // whole row twitched when the turn ended.
            let mark = if open { icons::Icon::CaretDown } else { icons::Icon::CaretRight };
            icons::paint(ui, rect.center(), mark, theme::ICON_CHEVRON, theme::muted());
            let label = if working { "正在工作" } else { "已完成" };
            // Not selectable, unlike the prose around it: the header is the row's control, and a
            // selectable label is a `click_and_drag` widget (that is how egui implements
            // drag-to-select text) sitting on top of the row that owns it.
            ui.add(egui::Label::new(egui::RichText::new(label).size(theme::TEXT_SMALL).color(theme::muted())).selectable(false));
            if working {
                // One of the panel's own glyphs (the design draws no such mark), turned by the painter.
                // It asks for the next frame itself, because a turn that is thinking has nothing else to
                // render.
                let angle = std::f32::consts::TAU
                    * (ui.input(|input| input.time) as f32 / theme::SPINNER_TURN_SECONDS).fract();
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(theme::ICON_CHEVRON, theme::ICON_CHEVRON),
                    egui::Sense::hover(),
                );
                icons::paint_turned(
                    ui,
                    rect.center(),
                    icons::Icon::CircleNotch,
                    theme::ICON_CHEVRON,
                    theme::muted(),
                    angle,
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(theme::SPINNER_FRAME_MS));
            }
        })
        .response;
    // The header is **a widget of its own**, not the layout's response made clickable. Measured, after
    // a click on this row did nothing: `.interact(Sense::click())` on the response produced a widget
    // the hit test never picked — `hovered` and `clicked` both false with the pointer inside its own
    // rectangle — while an explicit `ui.interact` over that same rectangle was hovered and clicked.
    // The row therefore highlighted under the pointer and **no click ever arrived**: a disclosure that
    // could be seen and not opened. Every other clickable row in this panel is registered this way.
    let row = ui.interact(content.rect, ui.id().with(("quorfloat-process", id)), egui::Sense::click());
    // The reader's click works at any time, including while the model is still working: that is the
    // whole point of folding it by default rather than by phase.
    if row.clicked() {
        ui.memory_mut(|memory| memory.data.insert_temp(id, !open));
    }
    if row.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if open {
        ui.add_space(theme::GAP_CLOSE);
        for row in held {
            process_line(ui, *row, markdown);
        }
        ui.add_space(theme::GAP_CLOSE);
    }
}

/// One row inside an open working disclosure.
///
/// **The reasoning is drawn as text, never behind a second disclosure.** The row is already inside
/// one: this disclosure *is* the thinking's fold, and `entry_ui` does not fold reasoning at all. A
/// fold inside it would be the same fact asked twice — and closed by default, so opening the working
/// would appear to reveal nothing (see `docs/progress.md` §5).
///
/// @param ui - where to draw.
/// @param row - the held row.
/// @param markdown - the viewer's cache.
fn process_line(ui: &mut egui::Ui, row: Held<'_>, markdown: &mut egui_commonmark::CommonMarkCache) {
    match row {
        // Only the reasoning: this line's reply is the answer, drawn below the disclosure.
        Held::Reasoning(entry) => {
            if let Entry::Assistant { blocks, .. } = entry {
                reasoning_ui(ui, &reasoning_text(blocks));
            }
        }
        // A working line, drawn whole. Its prose is the *narration* of the work — the answer is the
        // only line whose prose is drawn outside the disclosure — so dropping it here would lose the
        // model's own account of what it was doing.
        Held::Working(Entry::Assistant { blocks, streaming }) => {
            reasoning_ui(ui, &reasoning_text(blocks));
            let rest: Vec<Block> = blocks
                .iter()
                .filter(|block| !matches!(block, Block::Reasoning(_)))
                .cloned()
                .collect();
            if !rest.is_empty() {
                assistant_ui(ui, &rest, *streaming, markdown);
            }
        }
        // A tool result, a system line: nothing to take apart.
        Held::Working(entry) => entry_ui(ui, entry, markdown),
    }
}

/// The reasoning of one message, as text on screen.
///
/// @param ui - where to draw.
/// @param reasoning - the message's reasoning, or an empty string when it has none.
fn reasoning_ui(ui: &mut egui::Ui, reasoning: &str) {
    if reasoning.is_empty() {
        return;
    }
    ui.add_space(6.0);
    wrapped(
        ui,
        egui::RichText::new(reasoning)
            .size(theme::TEXT_META)
            .color(theme::muted()),
    );
}

/// The identity a turn's disclosure is remembered under.
///
/// **What the turn is, not what it currently holds.** While the model works, the held rows change every
/// frame — the live reasoning grows a token at a time — so a key over their text would be a new id every
/// frame, and a reader who opened the disclosure mid-stream would watch it snap shut under the pointer.
/// The question is what identifies a turn and does not change while it is being answered; a group with
/// no question (the transcript trims its oldest lines) falls back to its first row.
///
/// **The turn's position goes in as well, and it is not decoration.** Asking the same question twice —
/// retrying a prompt, which is ordinary — gives two turns with the same question, and a key of the
/// question alone therefore gave two disclosures *one widget id at two rectangles*: egui drew its red
/// `🔥 First use of widget ID … / Second use …` over the conversation (measured). The position is what
/// tells those two turns apart.
///
/// A position alone would be the wrong key, and so is the pair when the buffer trims: dropping the
/// oldest lines shifts every later turn, so a disclosure whose identity shifted no longer matches
/// anything and **forgets** its state — which is the tolerable failure. What must never happen is a
/// disclosure that jumps to another turn, and the question in the key is what prevents it.
///
/// @param at - where the turn starts among the transcript's lines.
/// @param question - the turn's question, when it has one.
/// @param held - the rows the disclosure holds.
/// @returns the egui id for its fold.
fn process_id(at: usize, question: Option<&Entry>, held: &[Held<'_>]) -> egui::Id {
    let seed = question
        .map(|entry| format!("{entry:?}"))
        .or_else(|| held.first().map(|row| format!("{row:?}")))
        .unwrap_or_default();
    egui::Id::new(("quorfloat-process", at, text_hash(&seed)))
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
    egui::Id::new(("quorfloat-copied", text_hash(source)))
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
    use crate::app::session::transcript::Entry;

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
            assistant_ui(ui, blocks, *streaming, markdown);
            // The bar replaces the bare "生成中…" this used to be: the design puts the turn's
            // status and the copy control on one line under the answer, and the status is what
            // says whether the answer is finished (`.q-answerbar`). It belongs to the *answer*:
            // see `assistant_ui` for why a line inside the working does not get one.
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
    }
}

/// The blocks of one assistant line, drawn as they are.
///
/// Separated from [`entry_ui`] because **a line inside the working disclosure is not an answer**: the
/// bar under an answer carries the turn's status and a copy control for the prose above it, and a
/// second one inside the disclosure is a copy button for something the reader is reading as a step of
/// the work. (It is not a cosmetic difference: the first version drew the bar for every assistant
/// line, so opening the working showed `回答完成 … 复制回答` in the middle of it.)
///
/// **The reasoning is not drawn here, and there is no disclosure on this line.** A model's
/// working-out belongs to the turn's disclosure (`split_turn` holds it), which is the *one* fold a
/// turn has: a second one here is what showed up on screen as `已完成` above `思考`.
///
/// @param ui - where to draw.
/// @param blocks - the line's blocks.
/// @param streaming - whether the line is still growing.
/// @param markdown - the viewer's cache.
fn assistant_ui(
    ui: &mut egui::Ui,
    blocks: &[Block],
    streaming: bool,
    markdown: &mut egui_commonmark::CommonMarkCache,
) {
    for block in blocks {
        match block {
            // Markdown, once the answer has stopped growing. While it streams the text
            // is drawn as it is: half a fence or an unclosed `**` renders as literal
            // punctuation for a moment and then reflows, and watching an answer
            // rearrange itself is worse than watching it arrive plainly.
            Block::Text(text) => {
                if streaming {
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
            // The turn's working disclosure draws it, and it is the only thing that does.
            // Drawing it here as well is the second fold this panel must never show.
            Block::Reasoning(_) => {}
            Block::Call { name, arguments } => {
                wrapped(ui, egui::RichText::new(format!("$ {name} {arguments}")).size(theme::TEXT_META).color(theme::muted()).monospace());
            }
        }
        ui.add_space(2.0);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::transcript::{Block, Entry};

    /// A question line.
    fn question(text: &str) -> Entry {
        Entry::User { text: text.to_owned() }
    }

    /// An assistant line with the given blocks.
    fn assistant(blocks: Vec<Block>) -> Entry {
        Entry::Assistant { blocks, streaming: false }
    }

    /// A tool result line.
    fn tool(text: &str) -> Entry {
        Entry::Tool { name: Some("bash".to_owned()), text: text.to_owned(), is_error: false }
    }

    /// A line of reasoning, as a message carries it.
    fn reasoning(text: &str) -> Block {
        Block::Reasoning(text.to_owned())
    }

    /// A tool call, as a message carries it.
    fn call(name: &str) -> Block {
        Block::Call { name: name.to_owned(), arguments: "{}".to_owned() }
    }

    /// An answer's prose.
    fn prose(text: &str) -> Block {
        Block::Text(text.to_owned())
    }

    /// Where a line sits in the turn, so an assertion can be about indexes rather than references.
    ///
    /// @param turn - the turn's lines.
    /// @param entry - one of them.
    /// @returns its index.
    fn position(turn: &[Entry], entry: &Entry) -> usize {
        turn.iter()
            .position(|line| std::ptr::eq(line, entry))
            .unwrap_or_else(|| panic!("{entry:?} is not a line of this turn"))
    }

    /// Every line the split names, as indexes, in no particular order.
    ///
    /// @param turn - the turn's lines.
    /// @returns one index per named part; a line held *and* answered appears twice.
    fn named(turn: &[Entry]) -> Vec<usize> {
        let split = split_turn(turn);
        let mut indexes: Vec<usize> = Vec::new();
        indexes.extend(split.question.map(|entry| position(turn, entry)));
        indexes.extend(split.held.iter().map(|row| match row {
            Held::Working(entry) | Held::Reasoning(entry) => position(turn, entry),
        }));
        indexes.extend(split.answer.iter().map(|entry| position(turn, entry)));
        indexes.sort_unstable();
        indexes
    }

    /// The text of everything a split puts behind the disclosure.
    ///
    /// @param turn - the turn's lines.
    /// @returns the held rows, as text.
    fn held_text(turn: &[Entry]) -> Vec<String> {
        let split = split_turn(turn);
        assert!(!split.held.is_empty(), "expected something to disclose");
        split.held.iter().map(|row| format!("{row:?}")).collect()
    }

    #[test]
    fn a_turns_working_folds_and_its_question_and_answer_do_not() {
        // The rule the whole change is about, asserted as a rule: what folds is everything the model
        // produced except its final answer, and what stays is the question and that answer.
        let turn = vec![
            question("看看这个仓库"),
            assistant(vec![reasoning("先看看目录结构"), call("bash")]),
            tool("README.md src"),
            // The answer carries reasoning of its own, which is the real shape of a thinking model's
            // reply — and the shape that used to put a second disclosure on screen.
            assistant(vec![reasoning("目录已经看过了"), prose("这个仓库只有 README 和 src。")]),
        ];
        let split = split_turn(&turn);
        assert_eq!(split.question, Some(&turn[0]), "the question is the turn's first line");
        assert_eq!(
            split.held,
            vec![Held::Working(&turn[1]), Held::Working(&turn[2]), Held::Reasoning(&turn[3])],
            "the working is the step, the tool result, and the answer's thinking",
        );
        assert_eq!(split.answer, vec![&turn[3]], "and the answer is the last assistant line");
        // The thing that matters: neither the question nor the answer's *reply* is inside the fold.
        let held = held_text(&turn);
        assert!(held.iter().any(|row| row.contains("先看看目录结构")), "{held:?}");
        assert!(held.iter().any(|row| row.contains("README.md src")), "{held:?}");
        assert!(held.iter().any(|row| row.contains("目录已经看过了")), "{held:?}");
        assert!(!held.iter().any(|row| row.contains("看看这个仓库")), "the question is out: {held:?}");
        // The answer's reply rides along inside the entry the fold holds — what keeps it off screen
        // is that the row is held for its *reasoning* (`Held::Reasoning`), which is the thing the
        // rendering test below can see and this one pins.
        assert!(
            !split.held.iter().any(|row| matches!(row, Held::Working(entry) if std::ptr::eq(*entry, &turn[3]))),
            "the answer is not held whole: {split:?}",
        );
    }

    #[test]
    fn a_message_that_ends_in_a_tool_call_is_working_not_an_answer() {
        // A message that carries a tool call has not answered anything yet, however much prose
        // precedes the call. Getting this wrong is what leaves the model's half-finished narration on
        // screen as if it were the answer.
        let turn = vec![
            question("跑一下测试"),
            assistant(vec![prose("我先看看测试文件，然后——"), call("bash")]),
        ];
        let split = split_turn(&turn);
        assert!(
            split.answer.is_empty(),
            "a reply that ends in a call has not answered: {:?}",
            split.answer,
        );
        assert!(!split.held.is_empty(), "it is working, so it has something to disclose");
    }

    #[test]
    fn a_turn_with_no_working_has_nothing_to_fold() {
        // A question and its answer, with no reasoning and no tools between them: the working *is* the
        // answer, and a disclosure over nothing is a control that does nothing.
        let turn = vec![
            question("你好"),
            assistant(vec![prose("你好，有什么可以帮你的？")]),
        ];
        let split = split_turn(&turn);
        assert!(split.held.is_empty(), "nothing to disclose: {split:?}");
        assert_eq!(split.answer, vec![&turn[1]], "the answer is still the answer");
    }

    #[test]
    fn a_blank_thought_is_not_something_to_disclose() {
        // Upstream's test is `block.text.trim() !== ''`, and a fold opened onto an empty thought is a
        // control that does nothing however it is labelled.
        let turn = vec![
            question("你好"),
            assistant(vec![reasoning("   \n"), prose("你好。")]),
        ];
        let split = split_turn(&turn);
        assert!(split.held.is_empty(), "an empty thought is not working: {split:?}");
        assert_eq!(split.answer, vec![&turn[1]]);
    }

    #[test]
    fn every_line_of_a_turn_is_drawn_exactly_once() {
        // The invariant behind a line that vanished into a fold: whatever a turn is made of, each of
        // its lines is named by some part of the split. The one line that may be named twice is the
        // answer, which is held for its reasoning and answered for its reply — one message split into
        // the part that is not the answer and the part that is, not a duplicate.
        let turn = vec![
            question("看看"),
            assistant(vec![reasoning("先看一眼"), call("bash")]),
            tool("README.md src"),
            assistant(vec![reasoning("记下来了"), prose("这个仓库只有 README 和 src。")]),
        ];
        assert_eq!(
            named(&turn),
            vec![0, 1, 2, 3, 3],
            "every line once, and the answer twice — held for its thinking, answered for its reply",
        );
    }

    #[test]
    fn a_group_before_the_first_question_is_drawn_once() {
        // A transcript can open with the model already talking: the buffer keeps the newest lines, so
        // a long conversation loses its first question. That group has no question, and it is still
        // drawn exactly once — the version before this one drew such a line a second time in the
        // question's place.
        let turn = vec![assistant(vec![reasoning("接着上次继续"), call("bash")])];
        let split = split_turn(&turn);
        assert_eq!(split.question, None, "there is no question in this group");
        assert_eq!(split.held, vec![Held::Working(&turn[0])], "its working is held");
        assert!(split.answer.is_empty(), "and it has answered nothing yet");
        assert_eq!(named(&turn), vec![0], "named once, not twice");
    }
}
