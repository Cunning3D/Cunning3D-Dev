use std::borrow::Cow;

use crate::GlobalProfiler;
use crate::NanoSecond;
use crate::NsSource;

use crate::ScopeDetails;
use crate::ScopeId;
use crate::StreamInfo;
use crate::StreamInfoRef;
use crate::fetch_add_scope_id;

/// Report a stream of profile data from a thread to the [`GlobalProfiler`] singleton.
/// This is used for internal purposes only
pub fn internal_profile_reporter(
    info: ThreadInfo,
    scope_details: &[ScopeDetails],
    stream_scope_times: &StreamInfoRef<'_>,
) {
    GlobalProfiler::lock().report(info, scope_details, stream_scope_times);
}

#[derive(Clone)]
struct StackFrame {
    scope_id: ScopeId,
    data: String,
    /// The offset returned by `begin_scope` when this scope was first opened.
    /// This matches the value stored in the user's `ProfilerScope`.
    original_offset: usize,
    /// The current offset in the stream. This may differ from `original_offset`
    /// if `yield_frame` has been called.
    current_offset: usize,
}

/// Collects profiling data for one thread
pub struct ThreadProfiler {
    stream_info: StreamInfo,
    scope_details: Vec<ScopeDetails>,
    /// Current depth.
    depth: usize,
    now_ns: NsSource,
    reporter: ThreadReporter,
    start_time_ns: Option<NanoSecond>,
    scope_stack: Vec<StackFrame>,
}

impl Default for ThreadProfiler {
    fn default() -> Self {
        Self {
            stream_info: Default::default(),
            scope_details: Default::default(),
            depth: 0,
            now_ns: crate::now_ns,
            reporter: internal_profile_reporter,
            start_time_ns: None,
            scope_stack: Default::default(),
        }
    }
}

impl ThreadProfiler {
    /// Explicit initialize with custom callbacks.
    ///
    /// If not called, each thread will use the default nanosecond source ([`crate::now_ns`])
    /// and report scopes to the global profiler ([`internal_profile_reporter`]).
    ///
    /// For instance, when compiling for WASM the default timing function ([`crate::now_ns`]) won't work,
    /// so you'll want to call `puffin::ThreadProfiler::initialize(my_timing_function, internal_profile_reporter);`.
    pub fn initialize(now_ns: NsSource, reporter: ThreadReporter) {
        ThreadProfiler::call(|tp| {
            tp.now_ns = now_ns;
            tp.reporter = reporter;
        });
    }

    /// Register a function scope.
    #[must_use]
    pub fn register_function_scope(
        &mut self,
        function_name: impl Into<Cow<'static, str>>,
        file_path: impl Into<Cow<'static, str>>,
        line_nr: u32,
    ) -> ScopeId {
        let new_id = fetch_add_scope_id();
        self.scope_details.push(
            ScopeDetails::from_scope_id(new_id)
                .with_function_name(function_name)
                .with_file(file_path)
                .with_line_nr(line_nr),
        );
        new_id
    }

    /// Register a named scope.
    #[must_use]
    pub fn register_named_scope(
        &mut self,
        scope_name: impl Into<Cow<'static, str>>,
        function_name: impl Into<Cow<'static, str>>,
        file_path: impl Into<Cow<'static, str>>,
        line_nr: u32,
    ) -> ScopeId {
        let new_id = fetch_add_scope_id();
        self.scope_details.push(
            ScopeDetails::from_scope_id(new_id)
                .with_scope_name(scope_name)
                .with_function_name(function_name)
                .with_file(file_path)
                .with_line_nr(line_nr),
        );
        new_id
    }

    /// Marks the beginning of the scope.
    /// Returns position where to write scope size once the scope is closed.
    #[must_use]
    pub fn begin_scope(&mut self, scope_id: ScopeId, data: &str) -> usize {
        self.depth += 1;

        let (offset, start_ns) = self
            .stream_info
            .stream
            .begin_scope(self.now_ns, scope_id, data);

        self.scope_stack.push(StackFrame {
            scope_id,
            data: data.to_owned(),
            original_offset: offset,
            current_offset: offset,
        });

        self.stream_info.range_ns.0 = self.stream_info.range_ns.0.min(start_ns);
        self.start_time_ns = Some(self.start_time_ns.unwrap_or(start_ns));

        offset
    }

    /// Marks the end of the scope.
    /// Returns the current depth.
    pub fn end_scope(&mut self, start_offset: usize) {
        let now_ns = (self.now_ns)();

        let current_offset = if let Some(frame) = self.scope_stack.pop() {
            if frame.original_offset != start_offset {
                eprintln!("puffin ERROR: Scope mismatch in end_scope");
            }
            frame.current_offset
        } else {
            eprintln!("puffin ERROR: Mismatched scope begin/end calls (empty stack)");
            start_offset
        };

        self.stream_info.depth = self.stream_info.depth.max(self.depth);
        self.stream_info.num_scopes += 1;
        self.stream_info.range_ns.1 = self.stream_info.range_ns.1.max(now_ns);

        if self.depth > 0 {
            self.depth -= 1;
        } else {
            eprintln!("puffin ERROR: Mismatched scope begin/end calls");
        }

        self.stream_info.stream.end_scope(current_offset, now_ns);

        if self.depth == 0 {
            // We have no open scopes.
            // This is a good time to report our profiling stream to the global profiler:
            let thread_name = std::thread::current().name().map(|s| s.to_owned()).unwrap_or_else(|| {
                format!("Thread {:?}", std::thread::current().id())
            });

            let info = ThreadInfo {
                start_time_ns: self.start_time_ns,
                name: thread_name,
            };
            (self.reporter)(
                info,
                &self.scope_details,
                &self.stream_info.as_stream_into_ref(),
            );

            self.scope_details.clear();
            self.stream_info.clear();
        }
    }

    /// Manually report the current frame data and start a new frame,
    /// without closing the currently open scopes.
    /// This is useful for long running tasks that you want to inspect progress of.
    pub fn yield_frame(&mut self) {
        if self.depth == 0 {
            return;
        }
        let now_ns = (self.now_ns)();

        // 1. Close all open scopes in the stream
        for frame in self.scope_stack.iter().rev() {
            self.stream_info
                .stream
                .end_scope(frame.current_offset, now_ns);
        }

        // 2. Report
        self.stream_info.depth = self.stream_info.depth.max(self.depth);
        self.stream_info.num_scopes += self.depth;
        self.stream_info.range_ns.1 = self.stream_info.range_ns.1.max(now_ns);

        let thread_name = std::thread::current().name().map(|s| s.to_owned()).unwrap_or_else(|| {
            format!("Thread {:?}", std::thread::current().id())
        });
        // println!("DEBUG: Puffin yielding frame on {}", thread_name);

        let info = ThreadInfo {
            start_time_ns: self.start_time_ns,
            name: thread_name,
        };
        (self.reporter)(
            info,
            &self.scope_details,
            &self.stream_info.as_stream_into_ref(),
        );

        self.scope_details.clear();
        self.stream_info.clear();

        // 3. Re-open scopes
        self.start_time_ns = Some(now_ns);

        for frame in &mut self.scope_stack {
            let (new_offset, _) = self
                .stream_info
                .stream
                .begin_scope(self.now_ns, frame.scope_id, &frame.data);
            frame.current_offset = new_offset;
        }
    }

    /// Do something with the thread local [`ThreadProfiler`]
    #[inline]
    pub fn call<R>(f: impl Fn(&mut Self) -> R) -> R {
        thread_local! {
            pub static THREAD_PROFILER: std::cell::RefCell<ThreadProfiler> = Default::default();
        }
        THREAD_PROFILER.with(|p| f(&mut p.borrow_mut()))
    }
}

/// Used to identify one source of profiling data.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ThreadInfo {
    /// Useful for ordering threads.
    pub start_time_ns: Option<NanoSecond>,
    /// Name of the thread
    pub name: String,
}

// Function interface for reporting thread local scope details.
// The scope details array will contain information about a scope the first time it is seen.
// The stream will always contain the scope timing details.
type ThreadReporter = fn(ThreadInfo, &[ScopeDetails], &StreamInfoRef<'_>);
