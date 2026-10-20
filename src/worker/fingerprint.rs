use crate::sentry_protocol::types::{SentryEvent, StackFrame};
use crate::util::hash::fingerprint_hash;

/// `mechanism.type` sentry-cocoa puts on app-hang events ("App Hanging" and
/// its "App Hang Fully Blocked"-style variants alike).
const APP_HANG_MECHANISM: &str = "AppHang";

/// Images whose frames show a hanging main thread waiting, never why: kernel
/// traps (`mach_msg2_trap`, `__psynch_cvwait`), pthread condition waits,
/// libdispatch semaphore/sync waits, synchronous XPC replies, and the
/// CoreFoundation run loop. Apple platforms report these frames as
/// `<redacted>`, so only the image can be matched. Derived from sentry-cocoa
/// macOS hang stacks.
const WAIT_IMAGES: &[&str] = &[
    "libsystem_kernel.dylib",
    "libsystem_pthread.dylib",
    "libdispatch.dylib",
    "libxpc.dylib",
    "CoreFoundation",
];

/// Symbolicated frames that wait rather than cause a hang: the nested Flutter
/// run loop macOS's `ResizeSynchronizer` spins while waiting for a frame, Dart
/// condition-variable waits (`dart::ConditionVariable::WaitMicros` during VM
/// shutdown), and libc++ plumbing such as an inlined
/// `std::__fl::__cxx_atomic_load`.
fn is_wait_function(function: &str) -> bool {
    function == "FlutterRunLoop.pollFlutterMessagesOnce"
        || function.contains("ConditionVariable::Wait")
        || function.starts_with("std::")
}

/// File name of a frame's image path, e.g. `SkyLight` for
/// `/System/Library/PrivateFrameworks/SkyLight.framework/Versions/A/SkyLight`.
fn image_name(package: &str) -> &str {
    package.rsplit_once('/').map_or(package, |(_, name)| name)
}

/// The frame an app hang is grouped, titled and attributed by.
#[derive(Clone, Copy)]
enum HangFrame<'a> {
    /// Innermost named frame that is neither a wait nor the app's entry point.
    Function {
        image: Option<&'a str>,
        function: &'a str,
    },
    /// No such frame: the image of the innermost non-wait frame, since Apple
    /// system frames arrive as `<redacted>`.
    Image(&'a str),
}

/// Finds the frame a hanging main thread is stuck in, walking its stack
/// (outermost frame first, as sent) from the top: the innermost named frame
/// that is neither a wait nor the entry point, else the image of the
/// innermost non-wait frame.
fn hang_frame(frames: &[StackFrame]) -> Option<HangFrame<'_>> {
    // The app's own image is in-app, so its outermost frame is the entry
    // point (`main`) every main-thread stack runs through, not a cause.
    let entry_point = frames.iter().position(|f| f.in_app == Some(true));
    let mut innermost_image = None;
    for (index, frame) in frames.iter().enumerate().rev() {
        let image = frame
            .package
            .as_deref()
            .map(image_name)
            .filter(|i| !i.is_empty());
        if image.is_some_and(|i| WAIT_IMAGES.contains(&i)) {
            continue;
        }
        let function = frame
            .function
            .as_deref()
            .filter(|f| !f.is_empty() && *f != "<redacted>");
        if function.is_some_and(is_wait_function) {
            continue;
        }
        if let Some(function) = function
            && Some(index) != entry_point
        {
            return Some(HangFrame::Function { image, function });
        }
        innermost_image = innermost_image.or(image);
    }
    innermost_image.map(HangFrame::Image)
}

/// A sentry-cocoa app-hang event. Grouping these by in-app frames puts every
/// hang in one issue, since the only in-app frame is usually the entry point.
struct AppHang<'a> {
    exception_type: &'a str,
    frame: HangFrame<'a>,
}

impl<'a> AppHang<'a> {
    /// sentry-cocoa sends the hanging main thread's stack as the exception
    /// stacktrace.
    fn from_event(event: &'a SentryEvent) -> Option<Self> {
        let exc = event.exception.as_ref()?.values.first()?;
        if exc.mechanism.as_ref()?.mechanism_type.as_deref() != Some(APP_HANG_MECHANISM) {
            return None;
        }
        Some(Self {
            exception_type: exc.exception_type.as_deref().unwrap_or("Error"),
            frame: hang_frame(&exc.stacktrace.as_ref()?.frames)?,
        })
    }

    fn fingerprint(&self) -> String {
        match self.frame {
            HangFrame::Function { image, function } => fingerprint_hash(&[
                "app-hang",
                self.exception_type,
                image.unwrap_or(""),
                function,
            ]),
            HangFrame::Image(image) => fingerprint_hash(&["app-hang", self.exception_type, image]),
        }
    }

    fn title(&self) -> String {
        let label = match self.frame {
            HangFrame::Function { function, .. } => function,
            HangFrame::Image(image) => image,
        };
        format!("{} in {label}", self.exception_type)
    }

    fn culprit(&self) -> String {
        match self.frame {
            HangFrame::Function {
                image: Some(image),
                function,
            } => format!("{image} in {function}"),
            HangFrame::Function {
                image: None,
                function,
            } => function.to_string(),
            HangFrame::Image(image) => image.to_string(),
        }
    }
}

/// Compute the grouping fingerprint for an event
pub fn compute_fingerprint(event: &SentryEvent) -> String {
    // 1. Client-provided fingerprint
    if let Some(fp) = &event.fingerprint
        && !fp.is_empty()
    {
        let parts: Vec<String> = fp
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        let parts_ref: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        return fingerprint_hash(&parts_ref);
    }

    // 2. App hang: the frame the main thread is stuck in
    if let Some(hang) = AppHang::from_event(event) {
        return hang.fingerprint();
    }

    // 3. Exception-based: type + in-app stacktrace
    if let Some(exc) = &event.exception
        && let Some(first) = exc.values.first()
    {
        let exc_type = first.exception_type.as_deref().unwrap_or("");
        let exc_value = first.value.as_deref().unwrap_or("");

        if let Some(st) = &first.stacktrace {
            let in_app_frames: Vec<String> = st
                .frames
                .iter()
                .filter(|f| f.in_app.unwrap_or(false))
                .map(|f| {
                    format!(
                        "{}:{}",
                        f.function.as_deref().unwrap_or("?"),
                        f.filename.as_deref().unwrap_or("?")
                    )
                })
                .collect();

            if !in_app_frames.is_empty() {
                let frames_str = in_app_frames.join("|");
                return fingerprint_hash(&[exc_type, &frames_str]);
            }
        }

        // Exception type + value only
        if !exc_type.is_empty() {
            return fingerprint_hash(&[exc_type, exc_value]);
        }
    }

    // 4. Message-based
    if let Some(msg) = event.message.as_deref() {
        let logger = event.logger.as_deref().unwrap_or("");
        return fingerprint_hash(&[msg, logger]);
    }

    if let Some(le) = &event.logentry
        && let Some(msg) = le.message.as_deref()
    {
        let logger = event.logger.as_deref().unwrap_or("");
        return fingerprint_hash(&[msg, logger]);
    }

    // 5. Fallback: hash of level + platform
    let level = event.level.as_deref().unwrap_or("error");
    let platform = event.platform.as_deref().unwrap_or("other");
    fingerprint_hash(&["unknown", level, platform])
}

/// Derive issue title from the event
pub fn derive_title(event: &SentryEvent) -> String {
    if let Some(hang) = AppHang::from_event(event) {
        return hang.title();
    }

    if let Some(exc) = &event.exception
        && let Some(first) = exc.values.first()
    {
        let t = first.exception_type.as_deref().unwrap_or("Error");
        let v = first.value.as_deref().unwrap_or("");
        if v.is_empty() {
            return t.to_string();
        }
        return format!("{t}: {v}");
    }

    if let Some(msg) = event.message.as_deref() {
        return msg.chars().take(200).collect();
    }

    if let Some(le) = &event.logentry
        && let Some(msg) = le.message.as_deref()
    {
        return msg.chars().take(200).collect();
    }

    "<untitled>".to_string()
}

/// Derive culprit (short location) from the event
pub fn derive_culprit(event: &SentryEvent) -> Option<String> {
    if let Some(hang) = AppHang::from_event(event) {
        return Some(hang.culprit());
    }

    if let Some(exc) = &event.exception {
        for ev in &exc.values {
            if let Some(st) = &ev.stacktrace
                && let Some(frame) = st.frames.iter().rev().find(|f| f.in_app.unwrap_or(false))
            {
                let func = frame.function.as_deref().unwrap_or("?");
                let file = frame.filename.as_deref().unwrap_or("?");
                return Some(format!("{file} in {func}"));
            }
        }
    }
    event.transaction.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentry_protocol::types::*;
    use serde_json::json;

    fn make_exception_event(exc_type: &str, value: &str, func: &str) -> SentryEvent {
        SentryEvent {
            exception: Some(ExceptionInterface {
                values: vec![ExceptionValue {
                    exception_type: Some(exc_type.to_string()),
                    value: Some(value.to_string()),
                    stacktrace: Some(Stacktrace {
                        frames: vec![StackFrame {
                            filename: Some("app.js".to_string()),
                            function: Some(func.to_string()),
                            lineno: Some(42),
                            in_app: Some(true),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_same_exception_same_fingerprint() {
        let e1 = make_exception_event("TypeError", "null ref", "handleClick");
        let e2 = make_exception_event("TypeError", "null ref", "handleClick");
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_different_exception_different_fingerprint() {
        let e1 = make_exception_event("TypeError", "null ref", "handleClick");
        let e2 = make_exception_event("RangeError", "out of bounds", "process");
        assert_ne!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_client_fingerprint_overrides() {
        let mut e1 = make_exception_event("TypeError", "a", "fn1");
        let mut e2 = make_exception_event("RangeError", "b", "fn2");
        e1.fingerprint = Some(vec![serde_json::Value::String("custom-group".to_string())]);
        e2.fingerprint = Some(vec![serde_json::Value::String("custom-group".to_string())]);
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_message_fingerprint() {
        let e1 = SentryEvent {
            message: Some("Connection timeout".to_string()),
            ..Default::default()
        };
        let e2 = SentryEvent {
            message: Some("Connection timeout".to_string()),
            ..Default::default()
        };
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));

        let e3 = SentryEvent {
            message: Some("Different message".to_string()),
            ..Default::default()
        };
        assert_ne!(compute_fingerprint(&e1), compute_fingerprint(&e3));
    }

    #[test]
    fn test_derive_title_exception() {
        let e = make_exception_event("TypeError", "Cannot read x", "fn");
        assert_eq!(derive_title(&e), "TypeError: Cannot read x");
    }

    #[test]
    fn test_derive_title_message() {
        let e = SentryEvent {
            message: Some("Hello world".to_string()),
            ..Default::default()
        };
        assert_eq!(derive_title(&e), "Hello world");
    }

    #[test]
    fn test_derive_title_empty() {
        let e = SentryEvent::default();
        assert_eq!(derive_title(&e), "<untitled>");
    }

    #[test]
    fn test_derive_culprit() {
        let e = make_exception_event("TypeError", "err", "handleClick");
        assert_eq!(
            derive_culprit(&e),
            Some("app.js in handleClick".to_string())
        );
    }

    #[test]
    fn test_derive_culprit_no_exception() {
        let e = SentryEvent {
            transaction: Some("/api/users".to_string()),
            ..Default::default()
        };
        assert_eq!(derive_culprit(&e), Some("/api/users".to_string()));
    }

    // Trimmed sentry-cocoa payloads of tracker events in issue #606 ("App
    // Hanging: App hanging for at least 3000 ms.", macOS), whose 5,163 events
    // all shared one fingerprint. Frames are outermost-first as sent; the
    // trimmed ones are system frames between the entry point and the hang.
    const DYLD: &str = "dyld";
    const PLEZY: &str = "/Applications/Plezy.app/Contents/MacOS/Plezy";
    const APP_DELEGATE: &str = "/Users/runner/work/plezy/plezy/macos/Runner/AppDelegate.swift";
    const APP_MAIN: &str = "$sSo21NSApplicationDelegateP6AppKitE4mainyyFZ5Plezy0cB0C_Ttg5";
    const APPKIT: &str = "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit";
    const HISERVICES: &str = "/System/Library/Frameworks/ApplicationServices.framework/Versions/A/Frameworks/HIServices.framework/Versions/A/HIServices";
    const SKYLIGHT: &str =
        "/System/Library/PrivateFrameworks/SkyLight.framework/Versions/A/SkyLight";
    const CORE_FOUNDATION: &str =
        "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation";
    const SECURITY: &str = "/System/Library/Frameworks/Security.framework/Versions/A/Security";
    const LIBDISPATCH: &str = "/usr/lib/system/libdispatch.dylib";
    const LIBXPC: &str = "/usr/lib/system/libxpc.dylib";
    const LIBSYSTEM_KERNEL: &str = "/usr/lib/system/libsystem_kernel.dylib";
    const FLUTTER_MACOS: &str = "/Applications/Plezy.app/Contents/Frameworks/FlutterMacOS.framework/Versions/A/FlutterMacOS";
    const SPARKLE: &str =
        "/Applications/Plezy.app/Contents/Frameworks/Sparkle.framework/Versions/B/Sparkle";
    /// Issue #606's fingerprint: "App Hanging" + the in-app `main` frame.
    const ISSUE_606_FINGERPRINT: &str =
        "f88d76544c14349571ff57f60f150d5dd153810bf42b4353269b44948fb99ab8";

    fn redacted(package: &str, instruction_addr: &str) -> serde_json::Value {
        json!({
            "function": "<redacted>",
            "in_app": false,
            "instruction_addr": instruction_addr,
            "package": package,
        })
    }

    fn app_main(instruction_addr: &str) -> serde_json::Value {
        json!({
            "filename": APP_DELEGATE,
            "function": APP_MAIN,
            "in_app": true,
            "instruction_addr": instruction_addr,
            "package": PLEZY,
        })
    }

    fn app_hang(frames: Vec<serde_json::Value>) -> SentryEvent {
        serde_json::from_value(json!({
            "platform": "cocoa",
            "exception": {"values": [{
                "type": "App Hanging",
                "value": "App hanging for at least 3000 ms.",
                "mechanism": {"type": "AppHang"},
                "stacktrace": {"frames": frames, "snapshot": true},
            }]},
        }))
        .unwrap()
    }

    /// Event 194462 (plezy@6145504): a live window resize waiting for a frame.
    fn resize_hang() -> SentryEvent {
        app_hang(vec![
            redacted(DYLD, "0x0000000180216b98"),
            app_main("0x00000001022fc4f8"),
            redacted(APPKIT, "0x000000018458b2dc"),
            redacted(APPKIT, "0x00000001845fa254"),
            json!({"filename": "flutter/shell/platform/darwin/macos/framework/Source/FlutterView.mm", "function": "-[FlutterView setFrameSize:]", "in_app": false, "instruction_addr": "0x00000001087a7300", "package": FLUTTER_MACOS}),
            json!({"filename": "<compiler-generated>", "function": "@objc ResizeSynchronizer.beginResize", "in_app": false, "instruction_addr": "0x00000001087f2054", "package": FLUTTER_MACOS}),
            json!({"filename": "/Volumes/Work/s/w/ir/cache/builder/engine/src/flutter/shell/platform/darwin/macos/framework/Source/ResizeSynchronizer.swift", "function": "ResizeSynchronizer.beginResize", "lineno": 113, "in_app": false, "instruction_addr": "0x00000001087f1edc", "package": FLUTTER_MACOS}),
            json!({"filename": "/Volumes/Work/s/w/ir/cache/builder/engine/src/flutter/shell/platform/darwin/macos/framework/Source/FlutterRunLoop.swift", "function": "FlutterRunLoop.pollFlutterMessagesOnce", "lineno": 162, "in_app": false, "instruction_addr": "0x00000001087f0b04", "package": FLUTTER_MACOS}),
            redacted(CORE_FOUNDATION, "0x00000001806a09e8"),
            redacted(CORE_FOUNDATION, "0x00000001806a1528"),
            redacted(CORE_FOUNDATION, "0x00000001806a2c0c"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000180575fa8"),
            redacted(LIBSYSTEM_KERNEL, "0x000000018057e764"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000180588338"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000180575c34"),
        ])
    }

    /// Event 190452 (plezy@7c38668): the same resize wait from a translocated
    /// app, other addresses, and an engine build without line info.
    fn resize_hang_translocated() -> SentryEvent {
        const APP: &str = "/private/var/folders/2f/jndmwnkd2bd71p1313924s0r0000gn/T/AppTranslocation/531A3DAF-7235-40A9-9714-486D3BD8EC81/d/Plezy.app";
        let flutter =
            format!("{APP}/Contents/Frameworks/FlutterMacOS.framework/Versions/A/FlutterMacOS");
        app_hang(vec![
            redacted(DYLD, "0x00000001866a7e00"),
            json!({"filename": APP_DELEGATE, "function": APP_MAIN, "in_app": true, "instruction_addr": "0x00000001002bfd1c", "package": format!("{APP}/Contents/MacOS/Plezy")}),
            redacted(APPKIT, "0x000000018af1b7b0"),
            redacted(&flutter, "0x00000001064f30f0"),
            json!({"function": "ResizeSynchronizer.beginResize", "in_app": false, "instruction_addr": "0x000000010653cff0", "package": flutter}),
            json!({"function": "ResizeSynchronizer.beginResize", "in_app": false, "instruction_addr": "0x000000010653ceb0", "package": flutter}),
            json!({"function": "FlutterRunLoop.pollFlutterMessagesOnce", "in_app": false, "instruction_addr": "0x000000010653bd80", "package": flutter}),
            redacted(CORE_FOUNDATION, "0x0000000186bf41c4"),
            redacted(CORE_FOUNDATION, "0x0000000186b219c4"),
            redacted(CORE_FOUNDATION, "0x0000000186b230d8"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000186a21fc0"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000186a2a9c0"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000186a34574"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000186a21c34"),
        ])
    }

    /// Event 194463 (plezy@b8cff51, macOS 26.6.2): blocked on WindowServer.
    fn window_server_hang() -> SentryEvent {
        app_hang(vec![
            redacted(DYLD, "0x0000000185d404e4"),
            app_main("0x00000001002b44f8"),
            redacted(APPKIT, "0x000000018a5c97b0"),
            redacted(APPKIT, "0x000000018b59585c"),
            redacted(SKYLIGHT, "0x000000018cc25e4c"),
            redacted(SKYLIGHT, "0x000000018cc25c8c"),
            redacted(SKYLIGHT, "0x000000018cc28f80"),
            redacted(LIBSYSTEM_KERNEL, "0x00000001860c7fc0"),
            redacted(LIBSYSTEM_KERNEL, "0x00000001860d09c0"),
            redacted(LIBSYSTEM_KERNEL, "0x00000001860da5a4"),
            redacted(LIBSYSTEM_KERNEL, "0x00000001860c7c34"),
        ])
    }

    /// Event 194441 (plezy@6145504, macOS 27.0.0): WindowServer via HIServices.
    fn window_server_hang_via_hiservices() -> SentryEvent {
        app_hang(vec![
            redacted(DYLD, "0x0000000196ddbe80"),
            app_main("0x00000001027004f8"),
            redacted(APPKIT, "0x000000019b7f3c4c"),
            redacted(HISERVICES, "0x000000019e7aadd0"),
            redacted(SKYLIGHT, "0x000000019db80b88"),
            redacted(SKYLIGHT, "0x000000019db4c03c"),
            redacted(SKYLIGHT, "0x000000019db4a238"),
            redacted(LIBSYSTEM_KERNEL, "0x000000019716cfac"),
            redacted(LIBSYSTEM_KERNEL, "0x0000000197175a28"),
            redacted(LIBSYSTEM_KERNEL, "0x000000019717f8e0"),
            redacted(LIBSYSTEM_KERNEL, "0x000000019716cc20"),
        ])
    }

    /// Event 194439 (plezy@6145504, x86_64): quitting waits for the Dart VM.
    fn quit_hang() -> SentryEvent {
        app_hang(vec![
            redacted(DYLD, "0x00007ff81a61e1c8"),
            app_main("0x000000010ca8f68c"),
            redacted(APPKIT, "0x00007ff81e9064cd"),
            json!({"filename": "flutter/runtime/dart_vm.cc", "function": "flutter::DartVM::~DartVM", "lineno": 521, "in_app": false, "instruction_addr": "0x0000000113d64557", "package": FLUTTER_MACOS}),
            json!({"filename": "flutter/runtime/dart_vm_initializer.cc", "function": "DartVMInitializer::Cleanup", "lineno": 135, "in_app": false, "instruction_addr": "0x0000000113d656ab", "package": FLUTTER_MACOS}),
            json!({"filename": "flutter/third_party/dart/runtime/vm/dart.cc", "function": "dart::Dart::Cleanup", "lineno": 570, "in_app": false, "instruction_addr": "0x0000000113dfc311", "package": FLUTTER_MACOS}),
            json!({"filename": "flutter/third_party/dart/runtime/vm/port.cc", "function": "dart::PortMap::Shutdown", "in_app": false, "instruction_addr": "0x0000000113e82add", "package": FLUTTER_MACOS}),
            json!({"filename": "flutter/third_party/libcxx/include/__atomic/support/c11.h", "function": "std::__fl::__cxx_atomic_load[abi:nn210000]<bool>", "lineno": 81, "in_app": false, "instruction_addr": "0x0000000113ea7c6d", "package": FLUTTER_MACOS}),
            json!({"filename": "flutter/third_party/dart/runtime/platform/synchronization_posix.cc", "function": "dart::ConditionVariable::WaitMicros", "lineno": 144, "in_app": false, "instruction_addr": "0x0000000113dc470f", "package": FLUTTER_MACOS}),
            redacted(LIBSYSTEM_KERNEL, "0x00007ff81a9c2712"),
        ])
    }

    /// Event 194244 (plezy@b8cff51): Sparkle's authorization check waiting on
    /// a synchronous XPC reply from Security.
    fn sparkle_hang() -> SentryEvent {
        app_hang(vec![
            redacted(DYLD, "0x0000000189f104e4"),
            app_main("0x00000001026e84f8"),
            redacted(APPKIT, "0x000000018e79c7b0"),
            json!({"function": "SPUSystemNeedsAuthorizationAccessForBundlePath", "in_app": false, "instruction_addr": "0x0000000106dee580", "package": SPARKLE}),
            redacted(SPARKLE, "0x0000000106ded698"),
            redacted(SECURITY, "0x000000018dd9aeec"),
            redacted(SECURITY, "0x000000018dd9b278"),
            redacted(LIBXPC, "0x0000000189fb3eb0"),
            redacted(LIBDISPATCH, "0x000000018a13b004"),
            redacted(LIBDISPATCH, "0x000000018a13ac64"),
            redacted(LIBSYSTEM_KERNEL, "0x000000018a297fc0"),
            redacted(LIBSYSTEM_KERNEL, "0x000000018a2a09c0"),
            redacted(LIBSYSTEM_KERNEL, "0x000000018a2aa5a4"),
            redacted(LIBSYSTEM_KERNEL, "0x000000018a297c34"),
        ])
    }

    #[test]
    fn test_app_hang_causes_get_distinct_fingerprints() {
        let fingerprints: Vec<String> = [
            resize_hang(),
            window_server_hang(),
            quit_hang(),
            sparkle_hang(),
        ]
        .iter()
        .map(compute_fingerprint)
        .collect();
        for (i, fp) in fingerprints.iter().enumerate() {
            assert_ne!(fp, ISSUE_606_FINGERPRINT);
            assert!(!fingerprints[..i].contains(fp), "{fingerprints:?}");
        }
    }

    #[test]
    fn test_app_hang_same_cause_same_fingerprint() {
        assert_eq!(
            compute_fingerprint(&resize_hang()),
            compute_fingerprint(&resize_hang_translocated())
        );
        assert_eq!(
            compute_fingerprint(&window_server_hang()),
            compute_fingerprint(&window_server_hang_via_hiservices())
        );
    }

    #[test]
    fn test_app_hang_title_and_culprit_name_the_cause() {
        let cases = [
            (
                resize_hang(),
                "App Hanging in ResizeSynchronizer.beginResize",
                "FlutterMacOS in ResizeSynchronizer.beginResize",
            ),
            (window_server_hang(), "App Hanging in SkyLight", "SkyLight"),
            (
                quit_hang(),
                "App Hanging in dart::PortMap::Shutdown",
                "FlutterMacOS in dart::PortMap::Shutdown",
            ),
            (
                sparkle_hang(),
                "App Hanging in SPUSystemNeedsAuthorizationAccessForBundlePath",
                "Sparkle in SPUSystemNeedsAuthorizationAccessForBundlePath",
            ),
        ];
        for (event, title, culprit) in cases {
            assert_eq!(derive_title(&event), title);
            assert_eq!(derive_culprit(&event).as_deref(), Some(culprit));
        }
    }

    /// Event 194428 of issue #2961 (iOS, plezy@6145504): app code blocked in an
    /// AudioSession XPC call is attributed to the app frame, not the system
    /// frames above it.
    #[test]
    fn test_app_hang_in_app_code_names_the_app_frame() {
        const PLEZY_IOS: &str = "/private/var/containers/Bundle/Application/809575CC-8D99-4954-B321-2E43C6AF92DB/Plezy.app/Plezy";
        const IOS_CORE_FOUNDATION: &str =
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
        const IOS_FOUNDATION: &str = "/System/Library/Frameworks/Foundation.framework/Foundation";
        let event = app_hang(vec![
            redacted(DYLD, "0x00000001964bb5b8"),
            json!({"function": "$sSo21UIApplicationDelegateP5UIKitE4mainyyFZ5Plezy03AppB0C_Ttg5", "in_app": true, "instruction_addr": "0x0000000102e8850c", "package": PLEZY_IOS}),
            redacted(
                "/System/Library/PrivateFrameworks/UIKitCore.framework/UIKitCore",
                "0x000000019abe3644",
            ),
            redacted(IOS_CORE_FOUNDATION, "0x000000019666a7f4"),
            redacted(LIBDISPATCH, "0x00000001c4b36044"),
            json!({"function": "thunk for @escaping @callee_guaranteed () -> ()", "in_app": true, "instruction_addr": "0x0000000102e6107c", "package": PLEZY_IOS}),
            json!({"function": "closure #1 in MpvPlayerPlugin.handlePipCall", "lineno": 205, "in_app": true, "instruction_addr": "0x0000000102e7d5a0", "package": PLEZY_IOS}),
            json!({"function": "MpvPipController.setup", "lineno": 128, "in_app": true, "instruction_addr": "0x0000000102e846e0", "package": PLEZY_IOS}),
            redacted(
                "/System/Library/PrivateFrameworks/AudioSession.framework/AudioSession",
                "0x00000001b5111960",
            ),
            redacted(IOS_CORE_FOUNDATION, "0x0000000196682bb0"),
            redacted(IOS_FOUNDATION, "0x0000000196b2e9c0"),
            redacted(IOS_FOUNDATION, "0x0000000196b49dc4"),
            redacted(LIBXPC, "0x00000001965e6ae0"),
            redacted(LIBDISPATCH, "0x00000001c4b42ce8"),
            redacted(LIBSYSTEM_KERNEL, "0x000000024df30074"),
            redacted(LIBSYSTEM_KERNEL, "0x000000024df2ccd4"),
        ]);
        assert_eq!(
            derive_title(&event),
            "App Hanging in MpvPipController.setup"
        );
        assert_eq!(
            derive_culprit(&event).as_deref(),
            Some("Plezy in MpvPipController.setup")
        );
    }

    /// Issue #931, a sentry-cocoa macOS crash (mechanism `mach`), keeps its
    /// production fingerprint, title and culprit.
    #[test]
    fn test_non_hang_events_keep_fingerprint_title_and_culprit() {
        let crash: SentryEvent = serde_json::from_value(json!({
            "platform": "cocoa",
            "exception": {"values": [{
                "type": "EXC_BAD_ACCESS",
                "value": "Exception 1, Code 1, Subcode 2728 >\nRogue memory write has corrupted memory.",
                "mechanism": {"type": "mach", "handled": false},
                "stacktrace": {"frames": [
                    {"function": "_pthread_wqthread", "in_app": false, "instruction_addr": "0x0000000194e65f9c", "package": "/usr/lib/system/libsystem_pthread.dylib"},
                    {"function": "_dispatch_client_callout4", "in_app": false, "instruction_addr": "0x0000000194cc25e8", "package": LIBDISPATCH},
                    {"function": "MTLDispatchListApply", "in_app": false, "instruction_addr": "0x00000001a1b3df44", "package": "/System/Library/Frameworks/Metal.framework/Versions/A/Metal"},
                    {"function": "___ZN28MVKPresentableSwapchainImage22presentCAMetalDrawableEPU27objcproto16MTLCommandBuffer11objc_object19MVKImagePresentInfo_block_invoke", "in_app": true, "instruction_addr": "0x00000001040b0e88", "package": PLEZY},
                    {"function": "_ZN12MVKSwapchain17beginPresentationERK19MVKImagePresentInfo", "in_app": true, "instruction_addr": "0x0000000104133714", "package": PLEZY},
                ]},
            }]},
        }))
        .unwrap();
        assert_eq!(
            compute_fingerprint(&crash),
            "8a949a57ef495e1b198641a191a51f8bbf7f72d30b9e3f725a4b187a52a92db8"
        );
        assert_eq!(
            derive_title(&crash),
            "EXC_BAD_ACCESS: Exception 1, Code 1, Subcode 2728 >\nRogue memory write has corrupted memory."
        );
        assert_eq!(
            derive_culprit(&crash).as_deref(),
            Some("? in _ZN12MVKSwapchain17beginPresentationERK19MVKImagePresentInfo")
        );

        // sha256("TypeError|handleClick:app.js")
        let e = make_exception_event("TypeError", "null ref", "handleClick");
        assert_eq!(
            compute_fingerprint(&e),
            "8215c78d31d4dc01c832394f14a3619d28a466cbe3e3e1567fe571cb92e27c33"
        );
    }

    /// Client fingerprints still win: issue #62's Android ANRs keep their
    /// production fingerprint, and an app hang with one uses it.
    #[test]
    fn test_client_fingerprint_beats_app_hang_grouping() {
        let anr: SentryEvent = serde_json::from_value(json!({
            "platform": "java",
            "fingerprint": ["{{ default }}", "foreground-anr"],
            "exception": {"values": [{
                "type": "ApplicationNotResponding",
                "value": "ANR",
                "module": "io.sentry.android.core",
                "mechanism": {"type": "AppExitInfo"},
                "thread_id": 1,
                "stacktrace": {"frames": [
                    {"function": "nativePollOnce", "module": "android.os.MessageQueue", "native": true},
                    {"function": "nativeDispatchPlatformMessage", "module": "io.flutter.embedding.engine.FlutterJNI", "native": true},
                ]},
            }]},
        }))
        .unwrap();
        assert_eq!(
            compute_fingerprint(&anr),
            "c284fdad6936c1c1b7e1f31a055393b53910dbb2081f3bbe3588db983e31042e"
        );

        let mut hang = resize_hang();
        hang.fingerprint = Some(vec![json!("custom-group")]);
        assert_eq!(
            compute_fingerprint(&hang),
            fingerprint_hash(&["custom-group"])
        );
    }
}
