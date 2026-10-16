//! Frame scripts in an exported SWF.
//!
//! A [`ScriptPlan`] lists, for every exported frame, its label and the statements of its frame scripts (already merged
//! across layers). Exporting it produces:
//!
//! - **`ActionScript` 2**: a `DoAction` tag in the frame, compiled by [`as2`];
//! - **`ActionScript` 3**: one `DoABC` tag and a `SymbolClass` tag in the first frame, compiled by [`abc`], plus the
//!   `ActionScript3` flag in the file attributes.
//!
//! Labels are written as `FrameLabel` tags, so `gotoAndPlay("label")` finds its frame.

use anim_core::num::round_f64_to_u32;
use anim_script::{Expr, Stmt, Version};
use serde::{Deserialize, Serialize};

use crate::movie::FrameItem;
use crate::tag::code;

pub mod abc;
pub mod as2;

/// What the script of one exported frame holds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlanFrame {
    /// The frame label, if the frame has one.
    pub label: Option<String>,
    /// The statements of every frame script on this frame, top layer first.
    pub stmts: Vec<Stmt>,
}

/// The labels and frame scripts of an export, one entry per exported frame in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScriptPlan {
    pub version: Version,
    pub frames: Vec<PlanFrame>,
}

impl ScriptPlan {
    /// True when no frame has a script.
    #[must_use]
    pub fn has_scripts(&self) -> bool {
        self.frames.iter().any(|f| !f.stmts.is_empty())
    }
}

/// The scripts of a plan, compiled to tags.
#[derive(Debug, Clone, Default)]
pub struct CompiledScripts {
    /// Per frame, the items to add (a `DoAction`, or for the first frame of an `ActionScript` 3 movie the code tags).
    pub frames: Vec<Vec<FrameItem>>,
    /// What the compilers could not express.
    pub notes: Vec<String>,
}

/// A 1-based frame number from a script literal, at least 1 and at most 65535.
pub(crate) fn frame_number(n: f64) -> u32 {
    round_f64_to_u32(n).clamp(1, u32::from(u16::MAX))
}

/// Compiles `plan`. For `ActionScript` 3 with `stop_last`, the last frame also calls `stop()`.
///
/// # Errors
/// [`crate::SwfError::ScriptTooLarge`] when an `ActionScript` 2 jump or function exceeds the format's 16-bit offsets.
pub fn compile(plan: &ScriptPlan, stop_last: bool) -> Result<CompiledScripts, crate::SwfError> {
    let mut out = CompiledScripts { frames: vec![Vec::new(); plan.frames.len()], notes: Vec::new() };
    match plan.version {
        Version::As2 => {
            for (items, frame) in out.frames.iter_mut().zip(&plan.frames) {
                if frame.stmts.is_empty() {
                    continue;
                }
                let (body, notes) = as2::compile(&frame.stmts)?;
                items.push(FrameItem::DoAction(body));
                out.notes.extend(notes);
            }
        }
        Version::As3 => {
            let stop = [Stmt::Expr(Expr::Call(Box::new(Expr::Ident("stop".to_owned())), Vec::new()))];
            let last = plan.frames.len().checked_sub(1);
            let mut stmts: Vec<(u32, Vec<Stmt>)> = Vec::new();
            for (i, frame) in plan.frames.iter().enumerate() {
                let mut s = frame.stmts.clone();
                if stop_last && Some(i) == last {
                    s.extend(stop.iter().cloned());
                }
                if !s.is_empty() {
                    stmts.push((u32::try_from(i).unwrap_or(u32::MAX), s));
                }
            }
            if !stmts.is_empty() {
                let refs: Vec<(u32, &[Stmt])> = stmts.iter().map(|(f, s)| (*f, s.as_slice())).collect();
                let (abc_bytes, notes) = abc::compile(&refs);
                out.notes.extend(notes);
                let mut do_abc = Vec::new();
                // Lazy initialisation flag, then the name of the code block.
                do_abc.extend_from_slice(&1u32.to_le_bytes());
                do_abc.extend_from_slice(b"frame_scripts\0");
                do_abc.extend_from_slice(&abc_bytes);
                // The main timeline is character 0.
                let mut symbols = Vec::new();
                symbols.extend_from_slice(&1u16.to_le_bytes());
                symbols.extend_from_slice(&0u16.to_le_bytes());
                symbols.extend_from_slice(abc::CLASS_NAME.as_bytes());
                symbols.push(0);
                if let Some(first) = out.frames.first_mut() {
                    first.push(FrameItem::Raw { code: code::DO_ABC, body: do_abc });
                    first.push(FrameItem::Raw { code: code::SYMBOL_CLASS, body: symbols });
                }
            }
        }
    }
    Ok(out)
}
