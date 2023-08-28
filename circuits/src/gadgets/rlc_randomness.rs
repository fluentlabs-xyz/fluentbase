use crate::{constraint_builder::Query, util::Field};
use halo2_proofs::{
    circuit::{Layouter, Value},
    plonk::{Challenge, ConstraintSystem, FirstPhase},
};

#[derive(Clone, Copy, Debug)]
pub struct RlcRandomness(pub Challenge);

impl RlcRandomness {
    pub fn configure<F: Field>(cs: &mut ConstraintSystem<F>) -> Self {
        // TODO: this is a hack so that we don't get a "'No Column<Advice> is
        // used in phase Phase(0) while allocating a new "Challenge usable after
        // phase Phase(0)" error.
        // Maybe we can fix this by deferring column allocation until the build call?
        let _ = cs.advice_column();

        Self(cs.challenge_usable_after(FirstPhase))
    }

    pub fn query<F: Field>(&self) -> Query<F> {
        Query::Challenge(self.0)
    }

    pub fn value<F: Field>(&self, layouter: &impl Layouter<F>) -> Value<F> {
        layouter.get_challenge(self.0)
    }
}
