use gimli::{
    BaseAddresses, CfaRule, Register, RegisterRule, UnwindContext, UnwindExpression, UnwindTableRow,
};
#[cfg(feature = "dwarf-expr")]
use gimli::{Evaluation, EvaluationResult, Location, Value};

use super::arch::*;
use super::find_fde::{self, FDEFinder, FDESearchResult};
use crate::abi::PersonalityRoutine;
use crate::arch::*;
use crate::util::*;

struct StoreOnStack;

// gimli's MSRV doesn't allow const generics, so we need to pick a supported array size.
const fn next_value(x: usize) -> usize {
    let supported = [0, 1, 2, 3, 4, 8, 16, 32, 64, 128];
    let mut i = 0;
    while i < supported.len() {
        if supported[i] >= x {
            return supported[i];
        }
        i += 1;
    }
    192
}

impl<O: gimli::ReaderOffset> gimli::UnwindContextStorage<O> for StoreOnStack {
    type Rules = [(Register, RegisterRule<O>); next_value(MAX_REG_RULES)];
    type Stack = [UnwindTableRow<O, Self>; 2];
}

#[cfg(feature = "dwarf-expr")]
impl<R: gimli::Reader> gimli::EvaluationStorage<R> for StoreOnStack {
    type Stack = [Value; 64];
    type ExpressionStack = [(R, R); 0];
    type Result = [gimli::Piece<R>; 1];
}

#[derive(Debug)]
pub struct Frame {
    fde_result: FDESearchResult,
    row: UnwindTableRow<usize, StoreOnStack>,
}

impl Frame {
    pub fn from_context(ctx: &Context, signal: bool) -> Result<Option<Self>, gimli::Error> {
        let mut ra = ctx[Arch::RA];

        // Reached end of stack
        if ra == 0 {
            return Ok(None);
        }

        // RA points to the *next* instruction, so move it back 1 byte for the call instruction.
        if !signal {
            ra -= 1;
        }

        let fde_result = match find_fde::get_finder().find_fde(ra as _) {
            Some(v) => v,
            None => return Ok(None),
        };
        let mut unwinder = UnwindContext::<_, StoreOnStack>::new_in();
        let row = fde_result
            .fde
            .unwind_info_for_address(
                &fde_result.eh_frame,
                &fde_result.bases,
                &mut unwinder,
                ra as _,
            )?
            .clone();

        Ok(Some(Self { fde_result, row }))
    }

    #[cfg(feature = "dwarf-expr")]
    fn evaluate_expression(
        &self,
        ctx: &Context,
        expr: UnwindExpression<usize>,
    ) -> Result<usize, gimli::Error> {
        let expr = expr.get(&self.fde_result.eh_frame).unwrap();
        let mut eval =
            Evaluation::<_, StoreOnStack>::new_in(expr.0, self.fde_result.fde.cie().encoding());
        let mut result = eval.evaluate()?;
        loop {
            match result {
                EvaluationResult::Complete => break,
                EvaluationResult::RequiresMemory { address, .. } => {
                    let value = unsafe { (address as usize as *const usize).read_unaligned() };
                    result = eval.resume_with_memory(Value::Generic(value as _))?;
                }
                EvaluationResult::RequiresRegister { register, .. } => {
                    let value = ctx[register];
                    result = eval.resume_with_register(Value::Generic(value as _))?;
                }
                EvaluationResult::RequiresRelocatedAddress(address) => {
                    let value = unsafe { (address as usize as *const usize).read_unaligned() };
                    result = eval.resume_with_memory(Value::Generic(value as _))?;
                }
                _ => unreachable!(),
            }
        }

        Ok(
            match eval
                .as_result()
                .last()
                .ok_or(gimli::Error::PopWithEmptyStack)?
                .location
            {
                Location::Address { address } => address as usize,
                _ => unreachable!(),
            },
        )
    }

    #[cfg(not(feature = "dwarf-expr"))]
    fn evaluate_expression(
        &self,
        _ctx: &Context,
        _expr: UnwindExpression<usize>,
    ) -> Result<usize, gimli::Error> {
        Err(gimli::Error::UnsupportedEvaluation)
    }

    pub fn adjust_stack_for_args(&self, ctx: &mut Context) {
        let size = self.row.saved_args_size();
        ctx[Arch::SP] = ctx[Arch::SP].wrapping_add(size as usize);
    }

    pub fn unwind(&self, ctx: &Context) -> Result<Context, gimli::Error> {
        let row = &self.row;
        let mut new_ctx = ctx.clone();

        let cfa = match *row.cfa() {
            CfaRule::RegisterAndOffset { register, offset } => {
                ctx[register].wrapping_add(offset as usize)
            }
            CfaRule::Expression(expr) => self.evaluate_expression(ctx, expr)?,
        };

        new_ctx[Arch::SP] = cfa as _;
        new_ctx[Arch::RA] = 0;

        for (reg, rule) in row.registers() {
            let value = match *rule {
                // For most registers, `Undefined` indicates the value does not need to
                // be preserved so the value content does not matter. However when RA is
                // `Undefined` it indicates that the unwinding is complete.
                RegisterRule::Undefined => 0,
                RegisterRule::SameValue => ctx[*reg],
                RegisterRule::Offset(offset) => unsafe {
                    *((cfa.wrapping_add(offset as usize)) as *const usize)
                },
                RegisterRule::ValOffset(offset) => cfa.wrapping_add(offset as usize),
                RegisterRule::Register(r) => ctx[r],
                RegisterRule::Expression(expr) => {
                    let addr = self.evaluate_expression(ctx, expr)?;
                    unsafe { *(addr as *const usize) }
                }
                RegisterRule::ValExpression(expr) => self.evaluate_expression(ctx, expr)?,
                RegisterRule::Architectural => unreachable!(),
                RegisterRule::Constant(value) => value as usize,
            };
            new_ctx[*reg] = value;
        }

        Ok(new_ctx)
    }

    pub fn bases(&self) -> &BaseAddresses {
        &self.fde_result.bases
    }

    pub fn personality(&self) -> Option<PersonalityRoutine> {
        self.fde_result
            .fde
            .personality()
            .map(|x| unsafe { deref_pointer(x) })
            .map(|x| unsafe { core::mem::transmute(x) })
    }

    pub fn lsda(&self) -> usize {
        self.fde_result
            .fde
            .lsda()
            .map(|x| unsafe { deref_pointer(x) })
            .unwrap_or(0)
    }

    pub fn initial_address(&self) -> usize {
        self.fde_result.fde.initial_address() as _
    }

    pub fn is_signal_trampoline(&self) -> bool {
        self.fde_result.fde.is_signal_trampoline()
    }
}
