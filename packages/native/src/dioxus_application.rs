use blitz_shell::{BlitzApplication, View, WindowConfig};
use dioxus_core::{provide_context, Element, ScopeId, VirtualDom};
use dioxus_history::{History, MemoryHistory};
use std::env;
use std::rc::Rc;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{WindowAttributes, WindowId};

use crate::assets::DioxusNativeNetProvider;
use crate::link_handler::DioxusNativeNavigationProvider;
use crate::DioxusNativeWindowRenderer;
use crate::contexts::NativeContext;
use crate::{contexts::DioxusNativeDocument, BlitzShellEvent, DocumentConfig, DioxusDocument};

/// Dioxus-native specific event type
pub enum DioxusNativeEvent {
    /// A hotreload event, basically telling us to update our templates.
    #[cfg(all(
        feature = "hot-reload",
        debug_assertions,
        not(target_os = "android"),
        not(target_os = "ios")
    ))]
    DevserverEvent(dioxus_devtools::DevserverMsg),

    /// Create a new head element from the Link and Title elements
    ///
    /// todo(jon): these should probabkly be synchronous somehow
    CreateHeadElement {
        window: WindowId,
        name: String,
        attributes: Vec<(String, String)>,
        contents: Option<String>,
    },
}

/// Event for creating a new window at runtime
#[derive(Debug)]
pub struct CreateWindowEvent {
    pub app: fn() -> Element,
    pub window_attributes: WindowAttributes,
}

// Safe because fn() -> Element is Copy/Send/Sync and WindowAttributes is Send + Sync
unsafe impl Send for CreateWindowEvent {}
unsafe impl Sync for CreateWindowEvent {}

pub struct DioxusNativeApplication {
    pending_window: Option<WindowConfig<DioxusNativeWindowRenderer>>,
    inner: BlitzApplication<DioxusNativeWindowRenderer>,
    proxy: EventLoopProxy<BlitzShellEvent>,
}

fn window_debug_enabled() -> bool {
    env::var("PINGO_WINDOW_DEBUG")
        .map(|value| {
            !matches!(
                value.as_str(),
                "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF"
            )
        })
        .unwrap_or(true)
}

fn window_debug(message: impl AsRef<str>) {
    if window_debug_enabled() {
        eprintln!("[window-debug] {}", message.as_ref());
    }
}

impl DioxusNativeApplication {
    pub fn new(
        proxy: EventLoopProxy<BlitzShellEvent>,
        config: WindowConfig<DioxusNativeWindowRenderer>,
    ) -> Self {
        Self {
            pending_window: Some(config),
            inner: BlitzApplication::new(proxy.clone()),
            proxy,
        }
    }

    pub fn add_window(&mut self, window_config: WindowConfig<DioxusNativeWindowRenderer>) {
        self.inner.add_window(window_config);
    }

    fn handle_blitz_shell_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &DioxusNativeEvent,
    ) {
        match event {
            #[cfg(all(
                feature = "hot-reload",
                debug_assertions,
                not(target_os = "android"),
                not(target_os = "ios")
            ))]
            DioxusNativeEvent::DevserverEvent(event) => match event {
                dioxus_devtools::DevserverMsg::HotReload(hotreload_message) => {
                    for window in self.inner.windows.values_mut() {
                        let doc = window.downcast_doc_mut::<DioxusDocument>();

                        // Apply changes to vdom
                        dioxus_devtools::apply_changes(&doc.vdom, hotreload_message);

                        // Reload changed assets
                        for asset_path in &hotreload_message.assets {
                            if let Some(url) = asset_path.to_str() {
                                doc.reload_resource_by_href(url);
                            }
                        }

                        window.poll();
                    }
                }
                dioxus_devtools::DevserverMsg::Shutdown => {
                    window_debug("devserver requested shutdown; exiting event loop");
                    event_loop.exit()
                }
                dioxus_devtools::DevserverMsg::FullReloadStart => {}
                dioxus_devtools::DevserverMsg::FullReloadFailed => {}
                dioxus_devtools::DevserverMsg::FullReloadCommand => {}
                _ => {}
            },

            DioxusNativeEvent::CreateHeadElement {
                name,
                attributes,
                contents,
                window,
            } => {
                if let Some(window) = self.inner.windows.get_mut(window) {
                    let doc = window.downcast_doc_mut::<DioxusDocument>();
                    doc.create_head_element(name, attributes, contents);
                    window.poll();
                }
            }

            // Suppress unused variable warning
            #[cfg(not(all(
                feature = "hot-reload",
                debug_assertions,
                not(target_os = "android"),
                not(target_os = "ios")
            )))]
            #[allow(unreachable_patterns)]
            _ => {
                let _ = event_loop;
                let _ = event;
            }
        }
    }
}

impl ApplicationHandler<BlitzShellEvent> for DioxusNativeApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "tracing")]
        tracing::debug!("Injecting document provider into all windows");

        if let Some(config) = self.pending_window.take() {
            let mut window = View::init(config, event_loop, &self.proxy);
            let renderer = window.renderer.clone();
            let window_id = window.window_id();
            let doc = window.downcast_doc_mut::<DioxusDocument>();

            doc.vdom.in_scope(ScopeId::ROOT, || {
                let shared: Rc<dyn dioxus_document::Document> =
                    Rc::new(DioxusNativeDocument::new(self.proxy.clone(), window_id));
                provide_context(shared);
            });

            // Add NativeContext for event loop proxy access
            let native_ctx = NativeContext::new(self.proxy.clone(), window_id);
            doc.vdom
                .in_scope(ScopeId::ROOT, move || provide_context(native_ctx));

            // Add history
            let history_provider: Rc<dyn History> = Rc::new(MemoryHistory::default());
            doc.vdom
                .in_scope(ScopeId::ROOT, move || provide_context(history_provider));

            // Add renderer
            doc.vdom
                .in_scope(ScopeId::ROOT, move || provide_context(renderer));

            // Queue rebuild
            doc.initial_build();

            // And then request redraw
            window.request_redraw();

            // todo(jon): we should actually mess with the pending windows instead of passing along the contexts
            self.inner.windows.insert(window_id, window);
            window_debug(format!(
                "initial window inserted: id={window_id:?}, total_windows={}",
                self.inner.windows.len()
            ));
        }

        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if matches!(&event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            let close_kind = if matches!(&event, WindowEvent::CloseRequested) {
                "CloseRequested"
            } else {
                "Destroyed"
            };
            let before = self.inner.windows.len();
            // Some compositors can destroy windows without first sending CloseRequested.
            // Drop our view entry for both events so stale windows aren't polled/redrawn.
            let removed = self.inner.windows.remove(&window_id);
            let after = self.inner.windows.len();
            window_debug(format!(
                "window event: kind={close_kind}, id={window_id:?}, removed={}, windows_before={before}, windows_after={after}",
                removed.is_some()
            ));
            drop(removed);
            if self.inner.windows.is_empty() {
                window_debug("window map is empty, exiting event loop");
                event_loop.exit();
            }
            return;
        }

        self.inner.window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: BlitzShellEvent) {
        match event {
            BlitzShellEvent::Embedder(ref arc_event) => {
                // Check for CreateWindowEvent first
                if let Some(create_event) = arc_event.downcast_ref::<CreateWindowEvent>() {

                    // Create the VirtualDom from the app function
                    let vdom = VirtualDom::new(create_event.app);
                    let requested_title = create_event.window_attributes.title.clone();

                    // Set up providers for asset loading
                    let net_provider = Some(DioxusNativeNetProvider::shared(self.proxy.clone()));

                    #[cfg(feature = "html")]
                    let html_parser_provider = Some(Arc::new(blitz_html::HtmlProvider) as _);
                    #[cfg(not(feature = "html"))]
                    let html_parser_provider = None;

                    let navigation_provider = Some(Arc::new(DioxusNativeNavigationProvider) as _);

                    // Create document with proper config for asset loading
                    let doc = DioxusDocument::new(
                        vdom,
                        DocumentConfig {
                            net_provider,
                            html_parser_provider,
                            navigation_provider,
                            ..Default::default()
                        },
                    );

                    // Create renderer
                    let renderer = DioxusNativeWindowRenderer::with_features_and_limits(None, None);

                    // Create window config
                    let config = WindowConfig::with_attributes(
                        Box::new(doc) as _,
                        renderer.clone(),
                        create_event.window_attributes.clone(),
                    );

                    // Create the window directly using View::init like BlitzApplication does
                    let mut window = View::init(config, event_loop, &self.proxy);
                    let window_id = window.window_id();

                    // Inject context providers into the new window's VirtualDom
                    let doc = window.downcast_doc_mut::<DioxusDocument>();

                    // Add document context
                    doc.vdom.in_scope(ScopeId::ROOT, || {
                        let shared: Rc<dyn dioxus_document::Document> =
                            Rc::new(DioxusNativeDocument::new(self.proxy.clone(), window_id));
                        provide_context(shared);
                    });

                    // Add NativeContext for event loop proxy access
                    let native_ctx = NativeContext::new(self.proxy.clone(), window_id);
                    doc.vdom
                        .in_scope(ScopeId::ROOT, move || provide_context(native_ctx));

                    // Add history
                    let history_provider: Rc<dyn History> = Rc::new(MemoryHistory::default());
                    doc.vdom
                        .in_scope(ScopeId::ROOT, move || provide_context(history_provider));

                    // Add renderer
                    doc.vdom
                        .in_scope(ScopeId::ROOT, move || provide_context(renderer));

                    // Build the document
                    doc.initial_build();

                    // Resume the window (initializes the renderer)
                    eprintln!("[dioxus-native] Resuming window...");
                    window.resume();
                    eprintln!("[dioxus-native] Window resumed");

                    // Poll to process any pending work
                    window.poll();

                    // Request redraw
                    window.request_redraw();
                    eprintln!("[dioxus-native] Window added to map");

                    // Add to windows map
                    self.inner.windows.insert(window_id, window);
                    window_debug(format!(
                        "dynamic window inserted: id={window_id:?}, requested_title={requested_title:?}, total_windows={}",
                        self.inner.windows.len()
                    ));

                    return;
                }

                // Check for DioxusNativeEvent
                if let Some(event) = arc_event.downcast_ref::<DioxusNativeEvent>() {
                    self.handle_blitz_shell_event(event_loop, event);
                    return;
                }

                // Fall through to inner handler
                self.inner.user_event(event_loop, BlitzShellEvent::Embedder(arc_event.clone()));
            }
            event => self.inner.user_event(event_loop, event),
        }
    }
}
