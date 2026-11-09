//! For an application's tests: a [`Recorder`] registered in the registry makes
//! every [`crate::Pdf`] built from it record each render and hand back a stub
//! instead of compiling, so a test asserts what was rendered, with what.
//!
//! ```no_run
//! # async fn t(db: laterite_core::Db, mut registry: laterite_core::Registry) {
//! use rainmill_pdf::testing::Recorder;
//!
//! let recorder = Recorder::new();
//! registry.add(recorder.clone());
//! // ... the code under test renders through Pdf::new(db, &registry) ...
//! recorder.assert_rendered_with("acme.shop.invoice", |data| data["number"] == "2026/7");
//! # }
//! ```

use std::sync::{Arc, Mutex};

use serde_json::Value;

/// The bytes a recorded render returns: a PDF header and trailer, nothing drawn.
pub const STUB: &[u8] = b"%PDF-1.7\n% rendered by rainmill_pdf::testing::Recorder\n%%EOF\n";

/// One render the recorder saw.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    /// The template's code.
    pub code: String,
    /// The data, with the registered variables merged in.
    pub data: Value,
    /// The locale asked for.
    pub locale: Option<String>,
}

/// Records renders instead of compiling them. Cheap to clone; every clone
/// shares one record.
#[derive(Debug, Clone, Default)]
pub struct Recorder(Arc<Mutex<Vec<Recorded>>>);

impl Recorder {
    /// An empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record(&self, recorded: Recorded) {
        if let Ok(mut all) = self.0.lock() {
            all.push(recorded);
        }
    }

    /// Every render so far, in order.
    pub fn renders(&self) -> Vec<Recorded> {
        self.0.lock().map(|all| all.clone()).unwrap_or_default()
    }

    /// Panics unless template `code` was rendered.
    pub fn assert_rendered(&self, code: &str) {
        let renders = self.renders();
        assert!(
            renders.iter().any(|r| r.code == code),
            "expected a render of {code}; rendered: {:?}",
            renders.iter().map(|r| &r.code).collect::<Vec<_>>()
        );
    }

    /// Panics unless template `code` was rendered with data `check` accepts.
    pub fn assert_rendered_with(&self, code: &str, check: impl Fn(&Value) -> bool) {
        let renders = self.renders();
        assert!(
            renders.iter().any(|r| r.code == code && check(&r.data)),
            "expected a render of {code} with matching data; rendered: {renders:?}"
        );
    }

    /// Panics if anything was rendered.
    pub fn assert_nothing_rendered(&self) {
        let renders = self.renders();
        assert!(
            renders.is_empty(),
            "expected no renders; rendered: {renders:?}"
        );
    }
}
