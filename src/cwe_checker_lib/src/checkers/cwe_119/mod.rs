//! This module implements a check for CWE-119: Buffer Overflow and its variants
//! CWE-125: Out-of-bounds Read and CWE-787: Out-of-bounds Write.
//!
//! Arrays or buffers of any kind are often accessed through indices.
//! If the index of an access is outside of the bounds of the buffer this can
//! lead to severe consequences.
//! In the case of out-of-bounds read accesses this often leads to exposure of
//! sensitive information to an attacker.
//! Out-of-bounds write accesses can often be used to hijack the control flow of
//! a program and thus may lead to arbitrary code execution.
//!
//! See <https://cwe.mitre.org/data/definitions/119.html> for a detailed
//! description.
//!
//! ## How the check works
//!
//! The check uses the results of the
//! [Pointer Inference analysis](`crate::analysis::pointer_inference`)
//! to check whether any memory accesses may point outside of the bounds of the
//! corresponding memory objects. Additionally, the check uses a lightweight
//! dataflow fixpoint computation to ensure that for each memory object only the
//! first access outside of its bounds is flagged as a CWE.
//!
//! Currently, the check is only partially interprocedural.
//! Bounds of parameter objects can be detected, but bounds of memory objects
//! created in called functions (other than the standard allocation functions)
//! will not be detected.
//!
//! ## False Positives
//!
//! - Any analysis imprecision of the Pointer Inference analysis may lead to
//!   false positive results in this check.
//! - If no exact bounds for a memory object could be inferred then the
//!   strictest (smallest) bounds found are used, which can lead to false
//!   positive warnings.
//!
//! ## False Negatives
//!
//! - In cases where the Pointer Inference analysis could not infer any bounds
//!   at all for the memory object or the access index this check generally
//!   assumes analysis imprecision as the culprit and will not flag them as
//!   CWEs. This leads to false negatives, especially in cases where the bounds
//!   directly depend on user input.
//! - The Pointer Inference analysis cannot distinguish different objects
//!   located on the same stack frame. Thus buffer overflows on the stack can
//!   only be detected if they may reach outside of the whole stack frame.
//!   This leads to false negatives, especially for buffer overflows caused by
//!   off-by-one bugs.
//! - For parameters of extern calls where a corresponding call stub is defined
//!   the analysis approximates size parameters as small as possible, which can
//!   lead to false negatives. Currently, analysis imprecision would lead to too
//!   many false positives if we would approximate by larger possible size
//!   parameters.
//! - For parameters of extern function calls without corresponding function
//!   stubs the check only checks whether the parameter itself may point outside
//!   of the boundaries of a memory object. But since we generally do not know
//!   what size the called function expects the pointed-to object to have this
//!   still may miss buffer overflows occuring in the called function.
//! - Right now the check only considers buffers on the stack or the heap, but
//!   not buffers in global memory. Thus corresponding overflows of buffers in
//!   global memory are not detected.
//! - Since the check is only partially interprocedural at the moment, it will
//!   miss object sizes of objects created in called functions. For example, if
//!   allocations are wrapped in simple wrapper functions, the analysis will
//!   miss overflows for corresponding objects, because it cannot determine
//!   their object sizes.

// FIXME: The current implementation uses path hints for memory object IDs to determine object sizes interprocedurally.
// But the number of path hint combinations can grow exponentially
// with the call depth to the actual allocation size of a callee-created object.
// This led to state explosion in the PointerInference and thus path hints are not longer provided by the PointerInference.
// But without the path hints that this analysis depended on, the check can only resolve sizes of parameter objects,
// but not of objects returned from called functions (other than the standard allocation functions).
// A future implementation needs a better way to determine object sizes interprocedurally,
// probably depending on several fixpoint computations to circumvent the state explosion problems
// that the old implementation is vulnerable to.
use super::prelude::*;

use crate::analysis::pointer_inference::Data;
use crate::prelude::*;
use crate::utils::log::LogThread;

mod context;
use context::Context;
mod state;
use state::State;
mod stubs;

cwe_module!("CWE119", "0.3", check_cwe);

/// Run the check for CWE-119: Buffer Overflows.
///
/// This function prepares the fixpoint computation that computes the CWE warnings by setting the start states for all function starts.
/// Then the fixpoint computation is executed.
/// Afterwards, the collected logs and CWE warnings are collected from a separate logging thread and returned.
pub fn check_cwe(
    analysis_results: &AnalysisResults,
    _config: &serde_json::Value,
    _debug_settings: &debug::Settings,
) -> WithLogs<Vec<CweWarning>> {
    let log_thread = LogThread::spawn(LogThread::collect_and_deduplicate);

    let context = Context::new(analysis_results, log_thread.get_msg_sender());

    let mut fixpoint_computation =
        crate::analysis::forward_interprocedural_fixpoint::create_computation(context, None);

    for (sub_tid, entry_node_of_sub) in
        crate::analysis::graph::get_entry_nodes_of_subs(analysis_results.control_flow_graph)
    {
        if let Some(function_sig) = analysis_results.function_signatures.unwrap().get(&sub_tid) {
            let fn_start_state = State::new(&sub_tid, function_sig, analysis_results.project);
            fixpoint_computation.set_node_value(
                entry_node_of_sub,
                crate::analysis::interprocedural_fixpoint_generic::NodeValue::Value(fn_start_state),
            );
        }
    }

    fixpoint_computation.compute_with_max_steps(100);

    let (logs, cwe_warnings) = log_thread.collect();

    WithLogs::new(cwe_warnings, logs)
}
