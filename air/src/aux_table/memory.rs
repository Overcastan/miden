use super::{EvaluationFrame, FieldElement, MEMORY_TRACE_OFFSET};
use crate::utils::{binary_not, is_binary, EvaluationResult};
use core::ops::Range;
use vm_core::utils::range as create_range;
use winter_air::TransitionConstraintDegree;

// CONSTANTS
// ================================================================================================

/// The number of constraints on the management of the memory co-processor.
pub const NUM_CONSTRAINTS: usize = 13;
/// The degrees of constraints on the management of the memory co-processor.
pub const CONSTRAINT_DEGREES: [usize; NUM_CONSTRAINTS] = [
    7, 6, 9, 8, // Constrain the values in the d inverse column.
    8, // Enforce values in ctx, addr, clk transition correctly.
    8, 8, 8, 8, // Enforce memory is initialized to zero.
    8, 8, 8,
    8, // Ensure next old values equal current new values when ctx and addr don't change.
];
/// The number of elements accessible in one read or write memory access.
const NUM_ELEMENTS: usize = 4;
/// Column to hold the context ID of the current memory context.
const CTX_COL_IDX: usize = MEMORY_TRACE_OFFSET;
/// Column to hold the memory address.
const ADDR_COL_IDX: usize = CTX_COL_IDX + 1;
/// Column for the clock cycle in which the memory operation occurred.
const CLK_COL_IDX: usize = ADDR_COL_IDX + 1;
/// Columns to hold the old values stored at a given memory context, address, and clock cycle prior
/// to the memory operation. When reading from a new address, these are initialized to zero. When
/// reading or updating previously accessed memory, these values are set to equal the "new" values
/// of the previous row in the trace.
const U_COL_RANGE: Range<usize> = create_range(CLK_COL_IDX + 1, NUM_ELEMENTS);
/// Columns to hold the new values stored at a given memory context, address, and clock cycle after
/// the memory operation.
const V_COL_RANGE: Range<usize> = create_range(U_COL_RANGE.end, NUM_ELEMENTS);
/// Column for the lower 16-bits of the delta between two consecutive context IDs, addresses, or
/// clock cycles.
const D0_COL_IDX: usize = V_COL_RANGE.end;
/// Column for the upper 16-bits of the delta between two consecutive context IDs, addresses, or
/// clock cycles.
const D1_COL_IDX: usize = D0_COL_IDX + 1;
/// Column for the inverse of the delta between two consecutive context IDs, addresses, or clock
/// cycles, used to enforce that changes are correctly constrained.
const D_INV_COL_IDX: usize = D1_COL_IDX + 1;

// MEMORY TRANSITION CONSTRAINTS
// ================================================================================================

/// Builds the transition constraint degrees for the memory co-processor.
pub fn get_transition_constraint_degrees() -> Vec<TransitionConstraintDegree> {
    CONSTRAINT_DEGREES
        .iter()
        .map(|&degree| TransitionConstraintDegree::new(degree))
        .collect()
}

/// Returns the number of transition constraints for the memory co-processor.
pub fn get_transition_constraint_count() -> usize {
    NUM_CONSTRAINTS
}

/// Enforces constraints for the memory co-processor.
pub fn enforce_constraints<E: FieldElement>(
    frame: &EvaluationFrame<E>,
    result: &mut [E],
    memory_flag: E,
) {
    // Constrain the values in the d inverse column.
    let mut index = enforce_d_inv(frame, result, memory_flag);

    // Enforce values in ctx, addr, clk transition correctly.
    index += enforce_delta(frame, &mut result[index..], memory_flag);

    // Constrain the memory values.
    enforce_values(frame, &mut result[index..], memory_flag);
}

// TRANSITION CONSTRAINT HELPERS
// ================================================================================================

/// A constraint evaluation function to enforce that the `d_inv` "delta inverse" column used to
/// constrain the delta between two consecutive contexts, addresses, or clock cycles is updated
/// correctly.
fn enforce_d_inv<E: FieldElement>(
    frame: &EvaluationFrame<E>,
    result: &mut [E],
    memory_flag: E,
) -> usize {
    let constraint_count = 4;

    result.agg_constraint(0, memory_flag, is_binary(frame.n0()));
    result.agg_constraint(1, memory_flag * frame.not_n0(), frame.ctx_change());
    result.agg_constraint(2, memory_flag * frame.not_n0(), is_binary(frame.n1()));
    result.agg_constraint(
        3,
        memory_flag * frame.not_n0() * frame.not_n1(),
        frame.addr_change(),
    );

    constraint_count
}

/// A constraint evaluation function to enforce that the delta between two consecutive context IDs,
/// addresses, or clock cycles is updated and decomposed into the `d1` and `d0` columns correctly.
fn enforce_delta<E: FieldElement>(
    frame: &EvaluationFrame<E>,
    result: &mut [E],
    memory_flag: E,
) -> usize {
    let constraint_count = 1;

    // If the context changed, include the difference.
    result.agg_constraint(0, memory_flag * frame.n0(), frame.ctx_change());
    // If the context is the same, include the address difference if it changed or else include the
    // clock change.
    result.agg_constraint(
        0,
        memory_flag * frame.not_n0(),
        frame.n1() * frame.addr_change() + frame.not_n1() * frame.clk_change(),
    );
    // Always subtract the delta. It should offset the other changes.
    result.agg_constraint(0, memory_flag, -frame.delta());

    constraint_count
}

/// A constraint evaluation function to enforce that memory is initialized to zero and that when
/// memory is accessed again the old values are always set to equal the previous new values.
fn enforce_values<E: FieldElement>(
    frame: &EvaluationFrame<E>,
    result: &mut [E],
    memory_flag: E,
) -> usize {
    let constraint_count = NUM_ELEMENTS * 2;

    for i in 0..NUM_ELEMENTS {
        // Memory must be initialized to zero.
        result.agg_constraint(i, memory_flag * frame.init_memory_flag(), frame.u_next(i));

        // The next old values must equal the current new values when ctx and addr don't change.
        result.agg_constraint(
            NUM_ELEMENTS + i,
            memory_flag * frame.copy_memory_flag(),
            frame.u_next(i) - frame.v(i),
        );
    }

    constraint_count
}

// MEMORY FRAME EXTENSION TRAIT
// ================================================================================================

/// Trait to allow easy access to column values and intermediate variables used in constraint
/// calculations for the Memory co-processor.
trait EvaluationFrameExt<E: FieldElement> {
    // --- Column accessors -----------------------------------------------------------------------

    /// The current context value.
    fn ctx(&self) -> E;
    /// The current address.
    fn addr(&self) -> E;
    /// The current clock cycle.
    fn clk(&self) -> E;
    /// The next clock cycle.
    fn clk_next(&self) -> E;
    /// The value from the specified index of the old values (0, 1, 2, 3) in the current row.
    fn u(&self, index: usize) -> E;
    /// The value from the specified index of the old values (0, 1, 2, 3) in the next row.
    fn u_next(&self, index: usize) -> E;
    /// The value from the specified index of the new values (0, 1, 2, 3) in the current row.
    fn v(&self, index: usize) -> E;
    /// The next value of the lower 16-bits of the delta value being tracked between two consecutive
    /// context IDs, addresses, or clock cycles.
    fn d0_next(&self) -> E;
    /// The next value of the upper 16-bits of the delta value being tracked between two consecutive
    /// context IDs, addresses, or clock cycles.
    fn d1_next(&self) -> E;
    /// The current value of the column tracking the inverse delta used for constraint evaluations.
    fn d_inv(&self) -> E;

    // --- Intermediate variables & helpers -------------------------------------------------------

    /// The change between the current value in the specified column and the next value, calculated
    /// as `next - current`.
    fn change(&self, column: usize) -> E;
    /// An intermediate variable to help constrain context change updates in the delta inverse
    /// column.
    fn n0(&self) -> E;
    /// `1 - n0`
    fn not_n0(&self) -> E;
    /// An intermediate variable to help constrain address changes in the delta inverse column when
    /// the context doesn't change.
    fn n1(&self) -> E;
    /// `1 - n1`
    fn not_n1(&self) -> E;
    /// The difference between the next context and the current context.
    fn ctx_change(&self) -> E;
    /// The difference between the next address and the current address.
    fn addr_change(&self) -> E;
    /// The difference between the next clock value and the current one, minus 1.
    fn clk_change(&self) -> E;
    /// The delta between two consecutive context IDs, addresses, or clock cycles.
    fn delta(&self) -> E;

    // --- Flags ----------------------------------------------------------------------------------

    /// A flag to indicate memory is being accessed for the first time and should be initialized.
    fn init_memory_flag(&self) -> E;
    /// A flag to indicate that memory is being re-accessed and the current new values should be
    /// copied to the next old values.
    fn copy_memory_flag(&self) -> E;
}

impl<E: FieldElement> EvaluationFrameExt<E> for &EvaluationFrame<E> {
    // --- Column accessors -----------------------------------------------------------------------

    fn ctx(&self) -> E {
        self.current()[CTX_COL_IDX]
    }
    fn addr(&self) -> E {
        self.next()[ADDR_COL_IDX]
    }
    fn clk(&self) -> E {
        self.current()[CLK_COL_IDX]
    }
    fn clk_next(&self) -> E {
        self.next()[CLK_COL_IDX]
    }
    fn u(&self, index: usize) -> E {
        self.current()[U_COL_RANGE.start + index]
    }
    fn u_next(&self, index: usize) -> E {
        self.next()[U_COL_RANGE.start + index]
    }
    fn v(&self, index: usize) -> E {
        self.current()[V_COL_RANGE.start + index]
    }
    fn d0_next(&self) -> E {
        self.next()[D0_COL_IDX]
    }
    fn d1_next(&self) -> E {
        self.next()[D1_COL_IDX]
    }
    fn d_inv(&self) -> E {
        self.current()[D_INV_COL_IDX]
    }

    // --- Intermediate variables & helpers -------------------------------------------------------

    fn change(&self, column: usize) -> E {
        self.next()[column] - self.current()[column]
    }

    fn n0(&self) -> E {
        self.change(CTX_COL_IDX) * self.d_inv()
    }

    fn not_n0(&self) -> E {
        binary_not(self.n0())
    }

    fn n1(&self) -> E {
        self.change(ADDR_COL_IDX) * self.d_inv()
    }

    fn not_n1(&self) -> E {
        binary_not(self.n1())
    }

    fn ctx_change(&self) -> E {
        self.change(CTX_COL_IDX)
    }

    fn addr_change(&self) -> E {
        self.change(ADDR_COL_IDX)
    }

    fn clk_change(&self) -> E {
        self.change(CLK_COL_IDX) - E::ONE
    }

    fn delta(&self) -> E {
        E::from(2_u32.pow(16)) * self.d1_next() + self.d0_next()
    }

    // --- Flags ----------------------------------------------------------------------------------

    fn init_memory_flag(&self) -> E {
        self.n0() + self.not_n0() * self.n1()
    }

    fn copy_memory_flag(&self) -> E {
        self.not_n0() * self.not_n1()
    }
}
