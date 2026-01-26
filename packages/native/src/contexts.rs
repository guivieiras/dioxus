use blitz_shell::BlitzShellEvent;
use dioxus_document::{Document, NoOpDocument};
use winit::{event_loop::EventLoopProxy, window::WindowId};

use crate::DioxusNativeEvent;

/// Public context providing access to the event loop proxy.
/// This allows components to send custom events to the application.
#[derive(Clone)]
pub struct NativeContext {
    proxy: EventLoopProxy<BlitzShellEvent>,
    window_id: WindowId,
}

impl NativeContext {
    /// Create a new NativeContext
    pub fn new(proxy: EventLoopProxy<BlitzShellEvent>, window_id: WindowId) -> Self {
        Self { proxy, window_id }
    }

    /// Get a clone of the event loop proxy
    pub fn proxy(&self) -> EventLoopProxy<BlitzShellEvent> {
        self.proxy.clone()
    }

    /// Get the window ID
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// Send a custom event to the event loop
    pub fn send_event<T: std::any::Any + Send + Sync + 'static>(&self, event: T) -> Result<(), winit::event_loop::EventLoopClosed<BlitzShellEvent>> {
        self.proxy.send_event(BlitzShellEvent::embedder_event(event))
    }
}

pub struct DioxusNativeDocument {
    pub(crate) proxy: EventLoopProxy<BlitzShellEvent>,
    pub(crate) window: WindowId,
}

impl DioxusNativeDocument {
    pub(crate) fn new(proxy: EventLoopProxy<BlitzShellEvent>, window: WindowId) -> Self {
        Self { proxy, window }
    }
}

impl Document for DioxusNativeDocument {
    fn eval(&self, _js: String) -> dioxus_document::Eval {
        NoOpDocument.eval(_js)
    }

    fn create_head_element(
        &self,
        name: &str,
        attributes: &[(&str, String)],
        contents: Option<String>,
    ) {
        let window = self.window;
        _ = self.proxy.send_event(BlitzShellEvent::embedder_event(
            DioxusNativeEvent::CreateHeadElement {
                name: name.to_string(),
                attributes: attributes
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.clone()))
                    .collect(),
                contents,
                window,
            },
        ));
    }

    fn set_title(&self, title: String) {
        self.create_head_element("title", &[], Some(title));
    }

    fn create_meta(&self, props: dioxus_document::MetaProps) {
        let attributes = props.attributes();
        self.create_head_element("meta", &attributes, None);
    }

    fn create_script(&self, props: dioxus_document::ScriptProps) {
        let attributes = props.attributes();
        self.create_head_element("script", &attributes, props.script_contents().ok());
    }

    fn create_style(&self, props: dioxus_document::StyleProps) {
        let attributes = props.attributes();
        self.create_head_element("style", &attributes, props.style_contents().ok());
    }

    fn create_link(&self, props: dioxus_document::LinkProps) {
        let attributes = props.attributes();
        self.create_head_element("link", &attributes, None);
    }

    fn create_head_component(&self) -> bool {
        true
    }
}
