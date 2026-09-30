use serde::Serialize;

/// A feature the engine doesn't implement yet. This lane's brief, item 3:
/// "Stub commands that exist in the contract's future but not the engine
/// yet (`compare`, `open_as_copy`, `send`, `reveal`) must return a typed
/// 'not available yet' error the UI shows plainly — never invent
/// results." The frontend (`src/lib/ipc.ts`) matches this exact shape
/// (`{ notAvailable: "<feature>" }`) and shows `feature` plainly.
#[derive(Debug, Serialize)]
pub struct NotAvailable {
    #[serde(rename = "notAvailable")]
    pub feature: String,
}

impl NotAvailable {
    pub fn new(feature: impl Into<String>) -> Self {
        Self {
            feature: feature.into(),
        }
    }
}
