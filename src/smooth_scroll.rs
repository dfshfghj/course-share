use crate::app::AppState;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, App, Bounds, Context, DispatchPhase, Element, ElementId,
    GlobalElementId, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, Pixels, Point,
    ScrollHandle, ScrollWheelEvent, UniformListScrollHandle, Window,
};
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct SmoothWheel<E: Element> {
    child: E,
    on_wheel: Rc<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>,
}

impl<E: Element> SmoothWheel<E> {
    pub(crate) fn new(
        child: E,
        on_wheel: Rc<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>,
    ) -> Self {
        Self { child, on_wheel }
    }
}

impl<E: Element> IntoElement for SmoothWheel<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: Element> Element for SmoothWheel<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = (E::PrepaintState, gpui_kit::Hitbox);

    fn id(&self) -> Option<ElementId> {
        self.child.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.child.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.child.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self
            .child
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        (state, hitbox)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            &mut prepaint.0,
            window,
            cx,
        );
        let hitbox = prepaint.1.clone();
        let on_wheel = self.on_wheel.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && hitbox.should_handle_scroll(window) {
                on_wheel(event, window, cx);
                cx.stop_propagation();
                window.prevent_default();
            }
        });
    }
}

#[derive(Clone)]
pub(crate) enum SmoothTarget {
    Handle(ScrollHandle),
}

impl SmoothTarget {
    pub(crate) fn uniform(handle: &UniformListScrollHandle) -> Self {
        Self::Handle(handle.0.borrow().base_handle.clone())
    }

    pub(crate) fn virtual_list(handle: &gpui_kit::base::VirtualListScrollHandle) -> Self {
        Self::Handle(handle.as_ref().clone())
    }

    pub(crate) fn offset_bounds(&self) -> (Pixels, Pixels) {
        match self {
            Self::Handle(handle) => {
                let max = handle.max_offset().y.max(px(0.));
                (handle.offset().y, -max)
            }
        }
    }

    fn set_offset_y(&self, y: Pixels) {
        match self {
            Self::Handle(handle) => {
                let x = handle.offset().x;
                handle.set_offset(Point { x, y });
            }
        }
    }

    fn scroll_down(&self, distance: Pixels) -> bool {
        let (current, min) = self.offset_bounds();
        let next = (current - distance).clamp(min, px(0.));
        if (next - current).abs() < px(0.01) {
            return false;
        }
        self.set_offset_y(next);
        true
    }
}

impl AppState {
    pub(crate) fn smooth_viewport(
        &self,
        id: &'static str,
        target: SmoothTarget,
        viewport: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        SmoothWheel {
            child: div().flex_1().min_w_0().h_full().flex().child(viewport),
            on_wheel: Rc::new(move |event, _, cx| {
                let target = target.clone();
                view.update(cx, |this, cx| this.smooth_wheel(id, target, event, cx));
            }),
        }
        .into_any_element()
    }

    pub(crate) fn smooth_wheel(
        &mut self,
        id: &'static str,
        target: SmoothTarget,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(18.)).y;
        let amount = -delta;
        if amount.abs() < px(0.01) {
            return;
        }
        let same = self
            .smooth_target
            .as_ref()
            .map(|(current, _)| *current == id)
            .unwrap_or(false);
        if !same {
            self.smooth_target = Some((id, target));
            self.smooth_pending = px(0.);
        }
        self.smooth_pending += amount;
        self.start_smooth_scroll(cx);
    }

    pub(crate) fn scroll_target_to_top(
        &mut self,
        id: &'static str,
        target: SmoothTarget,
        cx: &mut Context<Self>,
    ) {
        let (current, _) = target.offset_bounds();
        if current >= px(0.0) {
            return;
        }
        self.smooth_target = Some((id, target));
        self.smooth_pending = current;
        self.start_smooth_scroll(cx);
    }

    fn start_smooth_scroll(&mut self, cx: &mut Context<Self>) {
        if self.smooth_animating {
            return;
        }
        self.smooth_animating = true;
        let task = cx.spawn(async move |this, cx| {
            loop {
                let done = this
                    .update(cx, |this, cx| {
                        let Some((_, target)) = this.smooth_target.clone() else {
                            return true;
                        };
                        if this.smooth_pending.abs() < px(0.5) {
                            this.smooth_pending = px(0.);
                            this.smooth_target = None;
                            return true;
                        }
                        let step = this.smooth_pending * 0.3;
                        if target.scroll_down(step) {
                            this.smooth_pending -= step;
                            cx.notify();
                            false
                        } else {
                            this.smooth_pending = px(0.);
                            this.smooth_target = None;
                            true
                        }
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
            let _ = this.update(cx, |this, cx| {
                this.smooth_animating = false;
                if this.smooth_pending.abs() >= px(0.5) && this.smooth_target.is_some() {
                    this.start_smooth_scroll(cx);
                }
            });
        });
        self.smooth_scroll_task = Some(task);
    }
}
