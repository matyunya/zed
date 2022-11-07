pub mod shared_payloads;

use std::{any::Any, rc::Rc};

use collections::HashSet;
use gpui::{
    elements::{Empty, MouseEventHandler, Overlay},
    geometry::{rect::RectF, vector::Vector2F},
    scene::MouseDrag,
    CursorStyle, Element, ElementBox, EventContext, MouseButton, MutableAppContext, RenderContext,
    View, WeakViewHandle,
};

enum State<V: View> {
    Dragging {
        window_id: usize,
        position: Vector2F,
        region_offset: Vector2F,
        region: RectF,
        payload: Rc<dyn Any + 'static>,
        render: Rc<dyn Fn(Rc<dyn Any>, &mut RenderContext<V>) -> ElementBox>,
    },
    Canceled,
}

impl<V: View> Clone for State<V> {
    fn clone(&self) -> Self {
        match self {
            State::Dragging {
                window_id,
                position,
                region_offset,
                region,
                payload,
                render,
            } => Self::Dragging {
                window_id: window_id.clone(),
                position: position.clone(),
                region_offset: region_offset.clone(),
                region: region.clone(),
                payload: payload.clone(),
                render: render.clone(),
            },
            State::Canceled => State::Canceled,
        }
    }
}

pub struct DragAndDrop<V: View> {
    containers: HashSet<WeakViewHandle<V>>,
    currently_dragged: Option<State<V>>,
}

impl<V: View> Default for DragAndDrop<V> {
    fn default() -> Self {
        Self {
            containers: Default::default(),
            currently_dragged: Default::default(),
        }
    }
}

impl<V: View> DragAndDrop<V> {
    pub fn register_container(&mut self, handle: WeakViewHandle<V>) {
        self.containers.insert(handle);
    }

    pub fn currently_dragged<T: Any>(&self, window_id: usize) -> Option<(Vector2F, Rc<T>)> {
        self.currently_dragged.as_ref().and_then(|state| {
            if let State::Dragging {
                position,
                payload,
                window_id: window_dragged_from,
                ..
            } = state
            {
                if &window_id != window_dragged_from {
                    return None;
                }

                payload
                    .is::<T>()
                    .then(|| payload.clone().downcast::<T>().ok())
                    .flatten()
                    .map(|payload| (position.clone(), payload))
            } else {
                None
            }
        })
    }

    pub fn dragging<T: Any>(
        event: MouseDrag,
        payload: Rc<T>,
        cx: &mut EventContext,
        render: Rc<impl 'static + Fn(&T, &mut RenderContext<V>) -> ElementBox>,
    ) {
        let window_id = cx.window_id();
        cx.update_global::<Self, _, _>(|this, cx| {
            this.notify_containers_for_window(window_id, cx);

            if matches!(this.currently_dragged, Some(State::Canceled)) {
                return;
            }

            let (region_offset, region) = if let Some(State::Dragging {
                region_offset,
                region,
                ..
            }) = this.currently_dragged.as_ref()
            {
                (*region_offset, *region)
            } else {
                (
                    event.region.origin() - event.prev_mouse_position,
                    event.region,
                )
            };

            this.currently_dragged = Some(State::Dragging {
                window_id,
                region_offset,
                region,
                position: event.position,
                payload,
                render: Rc::new(move |payload, cx| {
                    render(payload.downcast_ref::<T>().unwrap(), cx)
                }),
            });
        });
    }

    pub fn render(cx: &mut RenderContext<V>) -> Option<ElementBox> {
        enum DraggedElementHandler {}
        cx.global::<Self>()
            .currently_dragged
            .clone()
            .and_then(|state| {
                match state {
                    State::Dragging {
                        window_id,
                        region_offset,
                        position,
                        region,
                        payload,
                        render,
                    } => {
                        if cx.window_id() != window_id {
                            return None;
                        }

                        dbg!("Rendered dragging state");
                        let position = position + region_offset;
                        Some(
                            Overlay::new(
                                MouseEventHandler::<DraggedElementHandler>::new(0, cx, |_, cx| {
                                    render(payload, cx)
                                })
                                .with_cursor_style(CursorStyle::Arrow)
                                .on_up(MouseButton::Left, |_, cx| {
                                    cx.defer(|cx| {
                                        cx.update_global::<Self, _, _>(|this, cx| {
                                            dbg!("Up with dragging state");
                                            this.finish_dragging(cx)
                                        });
                                    });
                                    cx.propagate_event();
                                })
                                .on_up_out(MouseButton::Left, |_, cx| {
                                    cx.defer(|cx| {
                                        cx.update_global::<Self, _, _>(|this, cx| {
                                            dbg!("Up out with dragging state");
                                            this.finish_dragging(cx)
                                        });
                                    });
                                })
                                // Don't block hover events or invalidations
                                .with_hoverable(false)
                                .constrained()
                                .with_width(region.width())
                                .with_height(region.height())
                                .boxed(),
                            )
                            .with_anchor_position(position)
                            .boxed(),
                        )
                    }

                    State::Canceled => {
                        dbg!("Rendered canceled state");
                        Some(
                            MouseEventHandler::<DraggedElementHandler>::new(0, cx, |_, _| {
                                Empty::new()
                                    .constrained()
                                    .with_width(0.)
                                    .with_height(0.)
                                    .boxed()
                            })
                            .on_up(MouseButton::Left, |_, cx| {
                                cx.defer(|cx| {
                                    cx.update_global::<Self, _, _>(|this, _| {
                                        dbg!("Up with canceled state");
                                        this.currently_dragged = None;
                                    });
                                });
                            })
                            .on_up_out(MouseButton::Left, |_, cx| {
                                cx.defer(|cx| {
                                    cx.update_global::<Self, _, _>(|this, _| {
                                        dbg!("Up out with canceled state");
                                        this.currently_dragged = None;
                                    });
                                });
                            })
                            .boxed(),
                        )
                    }
                }
            })
    }

    pub fn cancel_dragging<P: Any>(&mut self, cx: &mut MutableAppContext) {
        if let Some(State::Dragging {
            payload, window_id, ..
        }) = &self.currently_dragged
        {
            if payload.is::<P>() {
                let window_id = *window_id;
                self.currently_dragged = Some(State::Canceled);
                dbg!("Canceled");
                self.notify_containers_for_window(window_id, cx);
            }
        }
    }

    fn finish_dragging(&mut self, cx: &mut MutableAppContext) {
        if let Some(State::Dragging { window_id, .. }) = self.currently_dragged.take() {
            self.notify_containers_for_window(window_id, cx);
        }
    }

    fn notify_containers_for_window(&mut self, window_id: usize, cx: &mut MutableAppContext) {
        self.containers.retain(|container| {
            if let Some(container) = container.upgrade(cx) {
                if container.window_id() == window_id {
                    container.update(cx, |_, cx| cx.notify());
                }
                true
            } else {
                false
            }
        });
    }
}

pub trait Draggable {
    fn as_draggable<V: View, P: Any>(
        self,
        payload: P,
        render: impl 'static + Fn(&P, &mut RenderContext<V>) -> ElementBox,
    ) -> Self
    where
        Self: Sized;
}

impl<Tag> Draggable for MouseEventHandler<Tag> {
    fn as_draggable<V: View, P: Any>(
        self,
        payload: P,
        render: impl 'static + Fn(&P, &mut RenderContext<V>) -> ElementBox,
    ) -> Self
    where
        Self: Sized,
    {
        let payload = Rc::new(payload);
        let render = Rc::new(render);
        self.on_drag(MouseButton::Left, move |e, cx| {
            let payload = payload.clone();
            let render = render.clone();
            DragAndDrop::<V>::dragging(e, payload, cx, render)
        })
    }
}
