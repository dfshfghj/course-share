use gpui_component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode};
use gpui_kit::*;

pub(crate) fn vertical_rail<H: ScrollbarHandle + Clone + 'static>(
    id: &'static str,
    handle: &H,
) -> Div {
    div()
        .relative()
        .flex_none()
        .w(Scrollbar::width())
        .min_h_0()
        .child(
            div().absolute().inset_0().child(
                Scrollbar::vertical(handle)
                    .id(id)
                    .viewport_from_layout()
                    .mode(ScrollbarMode::Always),
            ),
        )
}

pub(crate) fn uniform_rail(id: &'static str, handle: &UniformListScrollHandle) -> Div {
    vertical_rail(id, &handle.0.borrow().base_handle)
}
