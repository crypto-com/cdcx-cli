pub mod cancel_order;
pub mod close_position;
pub mod oco_order;
pub mod otoco_order;
pub mod paper_order;
pub mod place_order;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowResult {
    Continue,
    Done,
    Cancel,
}

pub trait Workflow {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> WorkflowResult;
    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState);
    /// Called when the REST response for this workflow's submitted request arrives.
    /// Return `Continue` to keep the modal open (e.g. business-logic rejection — show the
    /// error so the user can retry), `Done` to close it on success, or `Cancel` to abort.
    /// Default keeps modal open — concrete workflows override to opt into auto-close.
    fn on_response(
        &mut self,
        _method: &str,
        _data: &serde_json::Value,
        _state: &mut AppState,
    ) -> WorkflowResult {
        WorkflowResult::Continue
    }
}

/// Draw a centered modal box with a border and title.
pub fn modal_area(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
