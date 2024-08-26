// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod conf;

use std::collections::HashSet;

pub use conf::init as init_conf;
pub use conf::lookup_conf_file;

use rustc_ast::AttrKind;
use rustc_middle::mir::{
    Constant, InlineAsmOperand, LocalDecl, Operand, Rvalue, Statement, StatementKind, Terminator,
    TerminatorKind,
};
use rustc_middle::query::Key;
use rustc_middle::ty::{ImplSubject, InstanceDef, TyCtxt, TyKind, WithOptConstParam};
use rustc_span::def_id::{DefId, LocalDefId, LOCAL_CRATE};
use rustc_span::Span;

const TOOL_NAME: &'static str = "component_access_control";
const CONTROLLED_ATTR: &'static str = "controlled";

pub fn enter_analysis<'tcx>(tcx: TyCtxt<'tcx>) {
    for mir_key in tcx.mir_keys(()) {
        check_body_mir(mir_key.clone(), tcx)
    }
}

fn check_body_mir(mir_key: LocalDefId, tcx: TyCtxt<'_>) {
    let def_id = WithOptConstParam::unknown(mir_key.to_def_id());
    // For const function/block, instance_mir returns mir_for_ctfe.
    // For normal function, instance_mir returns optimized_mir.
    let body = tcx.instance_mir(InstanceDef::Item(def_id));

    let mut checked_def_ids = HashSet::new();
    for basic_block_data in body.basic_blocks.iter() {
        // This check based on the assumption that any **DIRECT** visit to
        // static variables or functions can be found in Operand.
        // FIXME: is this true?
        for statement in &basic_block_data.statements {
            check_statement(statement, tcx, &mut checked_def_ids);
        }

        if let Some(terminator) = &basic_block_data.terminator {
            check_terminator(terminator, tcx, &mut checked_def_ids);
        }
    }

    // For some special cases, assign a function to a function pointer may not exist in statements,
    // while a local decl with type of the function exist. So we further check each local decl to
    // avoid missing any entry points.
    for local_decl in body.local_decls.iter() {
        check_local_decl(local_decl, tcx, &checked_def_ids)
    }
}

fn check_statement(statement: &Statement, tcx: TyCtxt<'_>, checked_def_ids: &mut HashSet<DefId>) {
    // FIXME: operand only exist in assign statement?
    let mut def_paths = Vec::new();
    if let StatementKind::Assign(assignment) = &statement.kind {
        let rvalue = &assignment.1;
        match rvalue {
            Rvalue::Use(operand)
            | Rvalue::Repeat(operand, _)
            | Rvalue::Cast(_, operand, _)
            | Rvalue::UnaryOp(_, operand)
            | Rvalue::ShallowInitBox(operand, _) => {
                check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
            }
            Rvalue::BinaryOp(_, two_operands) | Rvalue::CheckedBinaryOp(_, two_operands) => {
                check_invalid_operand(&two_operands.0, tcx, &mut def_paths, checked_def_ids);
                check_invalid_operand(&two_operands.1, tcx, &mut def_paths, checked_def_ids);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
                }
            }
            _ => {}
        }
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    emit_note(tcx, statement.source_info.span, crate_name, def_paths)
}

fn check_terminator(
    terminator: &Terminator,
    tcx: TyCtxt<'_>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    let mut def_paths = Vec::new();
    match &terminator.kind {
        TerminatorKind::SwitchInt { discr: operand, .. }
        | TerminatorKind::DropAndReplace { value: operand, .. }
        | TerminatorKind::Assert { cond: operand, .. }
        | TerminatorKind::Yield { value: operand, .. } => {
            check_invalid_operand(operand, tcx, &mut def_paths, checked_def_ids);
        }
        TerminatorKind::Call { func, args, .. } => {
            check_invalid_operand(func, tcx, &mut def_paths, checked_def_ids);
            for arg in args {
                check_invalid_operand(arg, tcx, &mut def_paths, checked_def_ids);
            }
        }
        TerminatorKind::InlineAsm { operands, .. } => {
            for asm_operand in operands {
                check_inline_asm_operand(&asm_operand, tcx, &mut def_paths, checked_def_ids);
            }
        }
        _ => {}
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    emit_note(tcx, terminator.source_info.span, crate_name, def_paths)
}

fn check_local_decl(local_decl: &LocalDecl<'_>, tcx: TyCtxt<'_>, checked_def_ids: &HashSet<DefId>) {
    let ty = local_decl.ty;
    let def_id = if let TyKind::FnDef(def_id, ..) = ty.kind() {
        // func def
        *def_id
    } else {
        return;
    };
    if checked_def_ids.contains(&def_id) {
        return;
    }
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    if let Some(def_path) = def_path_if_invalid_access(def_id, tcx) {
        emit_note(tcx, local_decl.source_info.span, crate_name, vec![def_path]);
    }
}

fn check_inline_asm_operand(
    asm_operand: &InlineAsmOperand<'_>,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    match asm_operand {
        InlineAsmOperand::In { value: operand, .. }
        | InlineAsmOperand::InOut {
            in_value: operand, ..
        } => {
            check_invalid_operand(operand, tcx, def_paths, checked_def_ids);
        }
        InlineAsmOperand::Const { value } | InlineAsmOperand::SymFn { value } => {
            check_constant(value, tcx, def_paths, checked_def_ids);
        }
        _ => {}
    }
}

/// check whether visiting the operand in local crate is valid.
/// if the operand is invalid, add the def_path to def_paths.
/// The operand is invalid only when following four points are all satisfied.
/// 1. The operand represents a static variable or a func(the first argument can not be self or its variants).
/// 2. The operand is not defined in local crate.
/// 3. The operand is marked with #[component_access_control::controlled]
/// 4. Local crate is not in the whitelist to visit the operand.
fn check_invalid_operand(
    operand: &Operand,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    if let Operand::Constant(constant) = operand {
        check_constant(&constant, tcx, def_paths, checked_def_ids);
    } else {
        return;
    };
}

fn check_constant(
    constant: &Constant<'_>,
    tcx: TyCtxt<'_>,
    def_paths: &mut Vec<String>,
    checked_def_ids: &mut HashSet<DefId>,
) {
    // get def_id of Constant and func
    let def_id = if let Some(def_id) = constant.check_static_ptr(tcx) {
        // static variable
        def_id
    } else {
        let ty = constant.ty();
        if let TyKind::FnDef(def_id, ..) = ty.kind() {
            // func def
            *def_id
        } else {
            return;
        }
    };
    checked_def_ids.insert(def_id);

    if let Some(def_path) = def_path_if_invalid_access(def_id, tcx) {
        def_paths.push(def_path);
    }
}

fn def_path_if_invalid_access(def_id: DefId, tcx: TyCtxt<'_>) -> Option<String> {
    if def_id.is_local() {
        return None;
    }
    if !contains_controlled_attr(def_id, tcx) {
        return None;
    }
    def_path_if_not_in_whitelist(def_id, tcx)
}

/// check whether the def_id is in white list.
/// If the def_id is **NOT** in white list, return the def_path
fn def_path_if_not_in_whitelist(def_id: DefId, tcx: TyCtxt<'_>) -> Option<String> {
    let crate_symbol = tcx.crate_name(LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    let def_path_str = def_path_for_def_id(tcx, def_id);
    if conf::CONFIG
        .get()
        .unwrap()
        .allow_access(crate_name, &def_path_str)
    {
        None
    } else {
        Some(def_path_str)
    }
}

/// if the def_id has attribute component_access_control::controlled, return true, else return false
fn contains_controlled_attr(def_id: DefId, tcx: TyCtxt<'_>) -> bool {
    for attr in tcx.get_attrs_unchecked(def_id) {
        if let AttrKind::Normal(normal_attr) = &attr.kind {
            let path_segments = &normal_attr.item.path.segments;
            if path_segments.len() != 2 {
                return false;
            }
            let segment_strs: Vec<_> = path_segments
                .iter()
                .map(|segment| segment.ident.as_str())
                .collect();
            if segment_strs[0] == TOOL_NAME && segment_strs[1] == CONTROLLED_ATTR {
                return true;
            }
        }
    }
    false
}

fn def_path_for_def_id(tcx: TyCtxt<'_>, def_id: DefId) -> String {
    match tcx.impl_of_method(def_id) {
        None => common_def_path_str(tcx, def_id),
        Some(impl_def_id) => def_path_str_for_impl(tcx, def_id, impl_def_id),
    }
}

/// def path for function, type, static variables and trait methods
fn common_def_path_str(tcx: TyCtxt<'_>, def_id: DefId) -> String {
    // This function is like def_path_debug_str without noisy info
    let def_path = tcx.def_path(def_id);
    let crate_name = tcx.crate_name(def_path.krate);
    format!("{}{}", crate_name, def_path.to_string_no_crate_verbose())
}

/// def path for impl, if the impl is not for trait.
fn def_path_str_for_impl(tcx: TyCtxt<'_>, def_id: DefId, impl_def_id: DefId) -> String {
    let item_name = tcx.item_name(def_id).to_string();
    let impl_subject = tcx.impl_subject(impl_def_id);
    if let ImplSubject::Inherent(impl_ty) = impl_subject {
        let impl_ty_def_id = impl_ty.ty_adt_id().expect("Method should impl an adt type");
        let impl_ty_name = common_def_path_str(tcx, impl_ty_def_id);
        return format!("{impl_ty_name}::{item_name}");
    }
    // impl trait goes here, which is impossible
    unreachable!()
}

fn emit_note(tcx: TyCtxt<'_>, span: Span, crate_name: &str, def_paths: Vec<String>) {
    if def_paths.len() > 0 {
        let sess = tcx.sess;
        const TITLE: &'static str = "access controlled entry point is disallowed";
        let def_path = def_paths.join(", ");
        let warning_message = format!("access {} in {}", def_path, crate_name);
        sess.struct_span_warn(span, TITLE)
            .note(warning_message)
            .emit();
    }
}
