use super::{
    super::constraint_builder::{
        AdviceColumn,
        AdviceColumnPhase2,
        BinaryColumn,
        ConstraintBuilder,
        FixedColumn,
        Query,
        SelectorColumn,
    },
    byte_bit::RangeCheck256Lookup,
    byte_representation::RlcLookup,
    is_zero::IsZeroGadget,
    rlc_randomness::RlcRandomness,
};
use ethers_core::types::U256;
use halo2_proofs::{
    arithmetic::{Field, FieldExt},
    circuit::{Region, Value},
    halo2curves::bn256::Fr,
    plonk::ConstraintSystem,
};
use itertools::Itertools;
use num_traits::Zero;

pub trait CanonicalRepresentationLookup {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 3];
}

#[derive(Clone)]
pub struct CanonicalRepresentationConfig {
    // Lookup columns
    value: AdviceColumn, // We're proving value.to_le_bytes()[i] = byte in this gadget
    index: FixedColumn,  // (0..32).repeat()
    byte: AdviceColumn,  // we need to prove that bytes form the canonical representation of value.
    rlc: AdviceColumnPhase2, /* Accumulated random linear combination of canonical representation
                          * bytes. */

    // Witness columns
    index_is_zero: SelectorColumn, // (0..32).repeat().map(|i| i == 0)
    // index_is_31: SelectorColumn, // (0..32).repeat().map(|i| i == 31)
    modulus_byte: FixedColumn, // (0..32).repeat().map(|i| Fr::MODULUS.to_be_bytes()[i])
    difference: AdviceColumn,  // modulus_byte - byte
    difference_is_zero: IsZeroGadget,
    differences_are_zero_so_far: BinaryColumn, /* difference[0] ... difference[index - 1] are
                                                * all 0. */
}

impl CanonicalRepresentationConfig {
    pub fn configure(
        cs: &mut ConstraintSystem<Fr>,
        cb: &mut ConstraintBuilder<Fr>,
        range_check: &impl RangeCheck256Lookup,
        randomness: &RlcRandomness,
    ) -> Self {
        let ([index_is_zero], [index, modulus_byte], [value, byte, difference]) =
            cb.build_columns(cs);
        let [rlc] = cb.second_phase_advice_columns(cs);

        let [differences_are_zero_so_far] = cb.binary_columns(cs);
        let difference_is_zero = IsZeroGadget::configure(cs, cb, difference);

        cb.assert_equal(
            "difference = modulus_byte - byte",
            difference.current(),
            modulus_byte.current() - byte.current(),
        );
        // TODO: just add an accumlator column?
        cb.condition(index_is_zero.current(), |cb| {
            cb.assert_equal(
                "every group of 32 bytes represent value",
                value.current(),
                (0..32)
                    .map(|i| byte.rotation(i))
                    .fold(Query::zero(), |acc, x| acc * 256 + x),
            );
            cb.assert(
                "differences_are_zero_so_far = 1 when index = 0",
                differences_are_zero_so_far.current(),
            );
            cb.assert_equal("???????", rlc.current(), byte.current());
        });
        cb.condition(!index_is_zero.current(), |cb| {
            cb.assert_equal(
                "value can only change when index = 0",
                value.current(),
                value.previous(),
            );
            cb.assert_equal(
                "differences_are_zero_so_far = difference == 0 && differences_are_zero_so_far.previous() when index != 0",
                differences_are_zero_so_far.current().into(),
                differences_are_zero_so_far
                    .previous()
                    .and(difference_is_zero.previous())
                    .into(),
            );
            cb.assert_equal(
                "???",
                rlc.current() ,
                rlc.previous() * randomness.query() + byte.current(),
            );
        });

        cb.add_lookup("0 <= byte < 256", [byte.current()], range_check.lookup());

        let is_first_nonzero_difference = differences_are_zero_so_far
            .current()
            .and(!difference_is_zero.current());
        cb.condition(is_first_nonzero_difference, |cb| {
            cb.add_lookup(
                "0 <= first nonzero difference < 256",
                // We know that the first nonzero difference is actually non-zero, but we don't
                // have a [1..255] range check.
                [difference.current()],
                range_check.lookup(),
            );
        });
        cb.condition(index_is_zero.rotation(-31), |cb| {
            cb.assert(
                "there is at least 1 nonzero difference",
                !(differences_are_zero_so_far
                    .current()
                    .and(difference_is_zero.current())),
            )
        });

        Self {
            value,
            index,
            byte,
            rlc,
            index_is_zero,
            modulus_byte,
            difference,
            difference_is_zero,
            differences_are_zero_so_far,
        }
    }

    pub fn assign<'a>(
        &self,
        region: &mut Region<'_, Fr>,
        randomness: Value<Fr>,
        values: impl IntoIterator<Item = &'a Fr>,
    ) {
        let modulus = U256::from_str_radix(Fr::MODULUS, 16).unwrap();
        let mut modulus_bytes = [0u8; 32];
        modulus.to_big_endian(&mut modulus_bytes);

        let mut offset = 0;
        // TODO: we add a final Fr::zero() to handle the always enabled selector. Add a default
        // assignment instead?
        for value in values.into_iter().copied().chain([Fr::zero()]) {
            let mut bytes = value.to_bytes();
            bytes.reverse();
            let mut differences_are_zero_so_far = true;
            let mut rlc = Value::known(Fr::zero());
            for (index, (byte, modulus_byte)) in bytes.iter().zip_eq(&modulus_bytes).enumerate() {
                self.byte.assign(region, offset, u64::from(*byte));
                self.modulus_byte
                    .assign(region, offset, u64::from(*modulus_byte));

                self.index
                    .assign(region, offset, u64::try_from(index).unwrap());
                if index.is_zero() {
                    self.index_is_zero.enable(region, offset);
                }

                let difference = Fr::from(u64::from(*modulus_byte)) - Fr::from(u64::from(*byte));
                self.difference.assign(region, offset, difference);
                self.difference_is_zero.assign(region, offset, difference);

                self.differences_are_zero_so_far.assign(
                    region,
                    offset,
                    differences_are_zero_so_far,
                );
                differences_are_zero_so_far &= difference.is_zero_vartime();

                self.value.assign(region, offset, value);

                rlc = rlc * randomness + Value::known(Fr::from(u64::from(*byte)));
                self.rlc.assign(region, offset, rlc);

                offset += 1
            }
        }
    }
}

impl CanonicalRepresentationLookup for CanonicalRepresentationConfig {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 3] {
        [
            self.value.current(),
            self.index.current(),
            self.byte.current(),
        ]
    }
}

impl RlcLookup for CanonicalRepresentationConfig {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 3] {
        [
            self.value.current(),
            self.rlc.current(),
            self.index.current(),
        ]
    }
}

#[cfg(test)]
mod test {
    use super::{super::byte_bit::ByteBitGadget, *};
    use halo2_proofs::{
        circuit::{Layouter, SimpleFloorPlanner},
        dev::MockProver,
        plonk::{Circuit, Error},
    };

    #[derive(Clone, Default, Debug)]
    struct TestCircuit {
        values: Vec<Fr>,
    }

    impl Circuit<Fr> for TestCircuit {
        type Config = (
            SelectorColumn,
            ByteBitGadget,
            RlcRandomness,
            CanonicalRepresentationConfig,
        );
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(cs: &mut ConstraintSystem<Fr>) -> Self::Config {
            let selector = SelectorColumn(cs.fixed_column());
            let mut cb = ConstraintBuilder::new(selector);

            let byte_bit = ByteBitGadget::configure(cs, &mut cb);
            let randomness = RlcRandomness::configure(cs);
            let canonical_representation =
                CanonicalRepresentationConfig::configure(cs, &mut cb, &byte_bit, &randomness);
            cb.build(cs);
            (selector, byte_bit, randomness, canonical_representation)
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<Fr>,
        ) -> Result<(), Error> {
            let (selector, byte_bit, rlc_randomness, canonical_representation) = config;
            let randomness = rlc_randomness.value(&layouter);
            layouter.assign_region(
                || "",
                |mut region| {
                    for offset in 0..256 {
                        selector.enable(&mut region, offset);
                    }
                    byte_bit.assign(&mut region);
                    canonical_representation.assign(&mut region, randomness, &self.values);
                    Ok(())
                },
            )
        }
    }

    #[test]
    fn test_canonical_representation() {
        let circuit = TestCircuit {
            values: vec![Fr::zero(), Fr::one(), Fr::from(256), Fr::zero() - Fr::one()],
        };
        let prover = MockProver::<Fr>::run(14, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()));
    }

    #[test]
    fn test_byte_ordering() {
        let value = Fr::from(258);
        let mut bytes = value.to_bytes();
        bytes.reverse();

        let mut expected = [0; 32];
        expected[30] = 1;
        expected[31] = 2;
        assert_eq!(bytes, expected);
    }
}
