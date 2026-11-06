//! The layout the tabs share: the heading levels, and the grid the job cards are laid out in.
use eframe::egui;

/// A tab's own heading ("Asset Packing", "Package Tools"): the top level of a tab's content.
const PAGE_HEADING_SIZE: f32 = 22.0;
/// A section heading inside a tab ("Hues and Lights", "Classic Client", "Logs"), over the cards it groups.
const SECTION_HEADING_SIZE: f32 = 19.0;
/// A card's title ("Hues", "World Lights", "Validate Packages"): the level under a section heading.
const CARD_TITLE_SIZE: f32 = 15.0;
/// The colour of the page and section headings.
const HEADING_COLOUR: egui::Color32 = egui::Color32::WHITE;
/// The colour of the card titles, a step below the headings.
const CARD_TITLE_COLOUR: egui::Color32 = egui::Color32::from_gray(220);

/// The narrowest a card is laid out beside another one.
const CARD_MIN_WIDTH: f32 = 560.0;
/// The gap between the cards of a grid, across and down.
const CARD_GRID_GAP: f32 = 10.0;
/// The cards in a row when the width allows two.
const WIDE_COLUMNS: usize = 2;
/// The cards in a row below that width.
const NARROW_COLUMNS: usize = 1;
/// The width from which the cards go two per row: two of the narrowest cards and the gap between them.
const CARD_GRID_TWO_COLUMN_MIN_WIDTH: f32 =
    WIDE_COLUMNS as f32 * CARD_MIN_WIDTH + (WIDE_COLUMNS - 1) as f32 * CARD_GRID_GAP;

pub(crate) fn page_heading(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).strong().size(PAGE_HEADING_SIZE).color(HEADING_COLOUR)
}

pub(crate) fn section_heading(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).strong().size(SECTION_HEADING_SIZE).color(HEADING_COLOUR)
}

pub(crate) fn card_title(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).strong().size(CARD_TITLE_SIZE).color(CARD_TITLE_COLOUR)
}

/// The cards in a row at `available_width`: two when two readable cards fit, else one.
fn card_grid_columns(available_width: f32) -> usize {
    if available_width >= CARD_GRID_TWO_COLUMN_MIN_WIDTH {
        WIDE_COLUMNS
    } else {
        NARROW_COLUMNS
    }
}

/// Lays cards out in rows, as `draw_card_grid` sets it up. Each card is drawn inside the grid's frame, and the cards of
/// a row share the row's height.
pub(crate) struct CardGrid<'a> {
    ui: &'a mut egui::Ui,
    id: egui::Id,
    frame: egui::Frame,
    columns: usize,
    column_width: f32,
    left: f32,
    row_top: f32,
    /// The cards of the row being laid out, their frames not closed yet.
    row: Vec<(egui::Ui, egui::containers::frame::Prepared)>,
    cards: usize,
    /// The space the rows laid out so far take.
    used: egui::Rect,
}

impl CardGrid<'_> {
    /// Draws the next card: `add_contents` fills its frame, which spans its column.
    pub(crate) fn card<R>(&mut self, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let column = self.cards % self.columns;
        let left = self.left + column as f32 * (self.column_width + CARD_GRID_GAP);
        let bottom = self.ui.max_rect().bottom().max(self.row_top);
        let max_rect = egui::Rect::from_min_max(
            egui::pos2(left, self.row_top),
            egui::pos2(left + self.column_width, bottom),
        );
        let mut cell = self.ui.new_child(
            egui::UiBuilder::new()
                .id_salt((self.id, self.cards))
                .max_rect(max_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let mut prepared = self.frame.begin(&mut cell);
        let content = &mut prepared.content_ui;
        content.set_min_width(content.available_width());
        let inner = add_contents(content);
        self.row.push((cell, prepared));
        self.cards += 1;
        if self.row.len() == self.columns {
            self.finish_row();
        }
        inner
    }

    /// Stretches the row's cards down to the bottom of the tallest one and closes their frames.
    fn finish_row(&mut self) {
        let bottom = self
            .row
            .iter()
            .map(|(_, prepared)| prepared.content_ui.min_rect().bottom())
            .fold(self.row_top, f32::max);
        for (mut cell, mut prepared) in self.row.drain(..) {
            prepared.content_ui.expand_to_include_y(bottom);
            let response = prepared.end(&mut cell);
            self.used = self.used.union(response.rect);
        }
        self.row_top = self.used.bottom() + CARD_GRID_GAP;
    }

    fn finish(mut self) {
        if !self.row.is_empty() {
            self.finish_row();
        }
        if self.cards > 0 {
            self.ui.advance_cursor_after_rect(self.used);
        }
    }
}

/// Lays out the cards `add_cards` draws through `CardGrid::card`, in order: two per row when the width allows, else one.
/// A last card alone in its row keeps the column's width.
pub(crate) fn draw_card_grid(
    ui: &mut egui::Ui,
    id_salt: &str,
    frame: egui::Frame,
    add_cards: impl FnOnce(&mut CardGrid<'_>),
) {
    let available_width = ui.available_width();
    let columns = card_grid_columns(available_width);
    let column_width = (available_width - (columns - 1) as f32 * CARD_GRID_GAP) / columns as f32;
    let top_left = ui.cursor().min;
    let id = ui.id().with(id_salt);
    let mut grid = CardGrid {
        ui,
        id,
        frame,
        columns,
        column_width,
        left: top_left.x,
        row_top: top_left.y,
        row: Vec::new(),
        cards: 0,
        used: egui::Rect::NOTHING,
    };
    add_cards(&mut grid);
    grid.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The height of the window the test grids are laid out in.
    const TEST_WINDOW_HEIGHT: f32 = 2000.0;
    /// Lines in the first, taller test card; the others have one.
    const TALL_CARD_LINES: usize = 4;

    /// Lays out `cards` cards at `width`, the first `TALL_CARD_LINES` lines tall and the others one line, and returns
    /// their frames in order, with the top of what follows the grid.
    fn lay_out(width: f32, cards: usize) -> (Vec<egui::Rect>, f32) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, TEST_WINDOW_HEIGHT))),
            ..Default::default()
        };
        let mut after_grid = 0.0;
        let output = egui::Context::default().run_ui(input, |ui| {
            let frame = egui::Frame::group(ui.style());
            draw_card_grid(ui, "test_grid", frame, |grid| {
                for card in 0..cards {
                    let lines = if card == 0 { TALL_CARD_LINES } else { 1 };
                    grid.card(|ui| {
                        for line in 0..lines {
                            ui.label(format!("line {line}"));
                        }
                    });
                }
            });
            after_grid = ui.cursor().top();
        });
        let frames = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => Some(rect.rect),
                _ => None,
            })
            .collect();
        (frames, after_grid)
    }

    #[test]
    fn two_cards_share_a_row_from_the_threshold_width_and_one_below_it() {
        assert_eq!(card_grid_columns(CARD_GRID_TWO_COLUMN_MIN_WIDTH), WIDE_COLUMNS);
        assert_eq!(card_grid_columns(CARD_GRID_TWO_COLUMN_MIN_WIDTH + CARD_MIN_WIDTH), WIDE_COLUMNS);
        assert_eq!(card_grid_columns(CARD_GRID_TWO_COLUMN_MIN_WIDTH - CARD_GRID_GAP), NARROW_COLUMNS);
    }

    #[test]
    fn wide_grids_put_two_cards_in_a_row_of_one_height() {
        let (frames, after_grid) = lay_out(CARD_GRID_TWO_COLUMN_MIN_WIDTH + CARD_MIN_WIDTH, 3);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].top(), frames[1].top(), "the first two cards share a row");
        assert_eq!(frames[0].height(), frames[1].height(), "and its height, though the first is taller");
        assert!(frames[1].left() > frames[0].right(), "the second card sits right of the first");
        assert!(frames[2].top() > frames[0].bottom(), "the third card starts the next row");
        assert_eq!(frames[2].left(), frames[0].left(), "in the first column");
        assert_eq!(frames[2].width(), frames[0].width(), "a card alone in its row keeps the column's width");
        assert!(frames[2].height() < frames[0].height(), "and its own height");
        assert!(after_grid >= frames[2].bottom(), "what follows the grid starts below it");
    }

    #[test]
    fn narrow_grids_put_one_card_per_row() {
        let (frames, after_grid) = lay_out(CARD_GRID_TWO_COLUMN_MIN_WIDTH - CARD_GRID_GAP, 2);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].left(), frames[1].left());
        assert!(frames[1].top() > frames[0].bottom());
        assert!(frames[1].height() < frames[0].height(), "a card alone in its row keeps its own height");
        assert!(after_grid >= frames[1].bottom());
    }
}
