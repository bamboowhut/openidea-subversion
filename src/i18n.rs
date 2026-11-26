use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::LazyLock;

use openidea_plugin::host;

static CHINESE: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../locales/zh-CN.json"))
        .expect("the Simplified Chinese catalog must be a JSON object of strings")
});

thread_local! {
    static CHINESE_UI: Cell<bool> = const { Cell::new(false) };
}

fn language() -> &'static str {
    match host::setting_string("language").as_deref() {
        Some("English") => "en",
        Some("Simplified Chinese") => "zh-CN",
        _ if host::locale()
            .split('-')
            .next()
            .is_some_and(|tag| tag.eq_ignore_ascii_case("zh")) =>
        {
            "zh-CN"
        }
        _ => "en",
    }
}

// Resolve the preference once per invocation, including nested SVN operations.
pub fn with_language<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CHINESE_UI.set(self.0);
        }
    }

    let _restore = Restore(CHINESE_UI.replace(language() == "zh-CN"));
    run()
}

pub fn tr(source: &str) -> &str {
    if CHINESE_UI.get() {
        CHINESE.get(source).map(String::as_str).unwrap_or(source)
    } else {
        source
    }
}

pub fn active_language() -> &'static str {
    if CHINESE_UI.get() { "zh-CN" } else { "en" }
}

pub fn interpolate(source: &str, args: &[&dyn Display]) -> String {
    let mut result = String::new();
    let mut parts = tr(source).split('{');
    result.push_str(parts.next().unwrap_or_default());
    for part in parts {
        if let Some((index, tail)) = part.split_once('}')
            && let Some(value) = index
                .parse::<usize>()
                .ok()
                .and_then(|index| args.get(index))
        {
            use std::fmt::Write;
            let _ = write!(result, "{value}{tail}");
        } else {
            result.push('{');
            result.push_str(part);
        }
    }
    result
}

macro_rules! trf {
    ($source:literal, $($value:expr),+ $(,)?) => {
        $crate::i18n::interpolate($source, &[$(&$value),+])
    };
}
pub(crate) use trf;

pub fn pending_source(label: &str) -> &str {
    // Older releases persisted the Chinese display label in recovery records.
    PENDING_LABELS
        .iter()
        .copied()
        .find(|source| {
            CHINESE
                .get(*source)
                .is_some_and(|translation| translation == label)
        })
        .unwrap_or(label)
}

pub fn pending_label(label: &str) -> &str {
    tr(pending_source(label))
}

const PENDING_LABELS: &[&str] = &[
    "Add",
    "Changelist",
    "Checkout",
    "Clean Up SVN Administrative Locks",
    "Commit",
    "Create branch or tag",
    "Delete Versioned Targets",
    "Delete property",
    "Import",
    "Lock",
    "Merge branch",
    "Move or rename",
    "Relocate",
    "Resolve conflict",
    "Revert SVN Changes",
    "Set property",
    "Share project",
    "Shelve",
    "Unlock",
    "Unshelve",
    "Update",
    "Switch branch",
];
