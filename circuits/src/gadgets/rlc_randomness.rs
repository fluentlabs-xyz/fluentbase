use crate::constraint_builder::Query;
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, Value},
    plonk::{Challenge, ConstraintSystem, FirstPhase},
};

#[derive(Clone, Copy, Debug)]
pub struct RlcRandomness(pub Challenge);

impl RlcRandomness {
    pub fn configure<F: FieldExt>(cs: &mut ConstraintSystem<F>) -> Self {
        // TODO: this is a hack so that we don't get a "'No Column<Advice> is
        // used in phase Phase(0) while allocating a new "Challenge usable after
        // phase Phase(0)" error.
        // Maybe we can fix this by deferring column allocation until the build call?
        let _ = cs.advice_column();

        Self(cs.challenge_usable_after(FirstPhase))
    }

    pub fn query<F: FieldExt>(&self) -> Query<F> {
        Query::Challenge(self.0)
    }

    pub fn value<F: FieldExt>(&self, layouter: &impl Layouter<F>) -> Value<F> {
        layouter.get_challenge(self.0)
    }
}
