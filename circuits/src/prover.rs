use halo2_proofs::{
    halo2curves::bn256::{Bn256, Fr, G1Affine},
    plonk::{create_proof, verify_proof, Circuit, ProvingKey, VerifyingKey},
    poly::{
        commitment::ParamsProver,
        kzg::{
            commitment::{KZGCommitmentScheme, ParamsKZG, ParamsVerifierKZG},
            multiopen::{ProverSHPLONK, VerifierSHPLONK},
            strategy::SingleStrategy,
        },
    },
    transcript::{
        Blake2bRead,
        Blake2bWrite,
        Challenge255,
        TranscriptReadBuffer,
        TranscriptWriterBuffer,
    },
};
use lazy_static::lazy_static;
use rand::{RngCore, SeedableRng};
use rand_xorshift::XorShiftRng;
use std::{collections::HashMap, sync::Mutex, time::Instant};

lazy_static! {
    static ref RNG: XorShiftRng = XorShiftRng::from_seed([
        0x59, 0x62, 0xbe, 0x5d, 0x76, 0x3d, 0x31, 0x8d, 0x17, 0xdb, 0x37, 0x32, 0x54, 0x06, 0xbc,
        0xe5,
    ]);
    static ref GEN_PARAMS: Mutex<HashMap<u32, ParamsKZG<Bn256>>> = Mutex::new(HashMap::new());
}

fn get_general_params(degree: u32) -> ParamsKZG<Bn256> {
    let mut map = GEN_PARAMS.lock().unwrap();
    match map.get(&degree) {
        Some(params) => params.clone(),
        None => {
            let params = ParamsKZG::<Bn256>::setup(degree, RNG.clone());
            map.insert(degree, params.clone());
            params
        }
    }
}

fn test_actual<C: Circuit<Fr>>(
    circuit: C,
    instance: Vec<Vec<Fr>>,
    proving_key: ProvingKey<G1Affine>,
    degree: u32,
) -> u64 {
    fn test_gen_proof<C: Circuit<Fr>, R: RngCore>(
        rng: R,
        circuit: C,
        general_params: &ParamsKZG<Bn256>,
        proving_key: &ProvingKey<G1Affine>,
        mut transcript: Blake2bWrite<Vec<u8>, G1Affine, Challenge255<G1Affine>>,
        instances: &[&[Fr]],
    ) -> Vec<u8> {
        create_proof::<
            KZGCommitmentScheme<Bn256>,
            ProverSHPLONK<'_, Bn256>,
            Challenge255<G1Affine>,
            R,
            Blake2bWrite<Vec<u8>, G1Affine, Challenge255<G1Affine>>,
            C,
        >(
            general_params,
            proving_key,
            &[circuit],
            &[instances],
            rng,
            &mut transcript,
        )
        .expect("proof generation should not fail");

        transcript.finalize()
    }

    fn test_verify(
        general_params: &ParamsKZG<Bn256>,
        verifier_params: &ParamsKZG<Bn256>,
        verifying_key: &VerifyingKey<G1Affine>,
        proof: &[u8],
        instances: &[&[Fr]],
    ) {
        let mut verifier_transcript = Blake2bRead::<_, G1Affine, Challenge255<_>>::init(proof);
        let strategy = SingleStrategy::new(general_params);

        verify_proof::<
            KZGCommitmentScheme<Bn256>,
            VerifierSHPLONK<'_, Bn256>,
            Challenge255<G1Affine>,
            Blake2bRead<&[u8], G1Affine, Challenge255<G1Affine>>,
            SingleStrategy<'_, Bn256>,
        >(
            verifier_params,
            verifying_key,
            strategy,
            &[instances],
            &mut verifier_transcript,
        )
        .expect("failed to verify circuit");
    }

    let general_params = get_general_params(degree);
    // println!("general params: {:?}", general_params);
    let verifier_params: ParamsVerifierKZG<Bn256> = general_params.verifier_params().clone();
    // println!("verifier params: {:?}", verifier_params);

    let transcript = Blake2bWrite::<_, G1Affine, Challenge255<_>>::init(vec![]);

    // change instace to slice
    let instance: Vec<&[Fr]> = instance.iter().map(|v| v.as_slice()).collect();
    // println!("instance: {:?}", instance);

    let start = Instant::now();
    let proof = test_gen_proof(
        RNG.clone(),
        circuit,
        &general_params,
        &proving_key,
        transcript,
        &instance,
    );
    let elapsed = start.elapsed();
    // println!("proof: {:?}", proof);

    let verifying_key = proving_key.get_vk();
    test_verify(
        &general_params,
        &verifier_params,
        verifying_key,
        &proof,
        &instance,
    );
    // println!("proof verified");

    elapsed.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluentbase_circuit::FluentbaseCircuit;
    use fluentbase_runtime::Runtime;
    use fluentbase_rwasm::{
        instruction_set,
        rwasm::{Compiler, ImportLinker, InstructionSet},
    };
    use halo2_proofs::plonk::{keygen_pk, keygen_vk};

    fn gen_proof_verify(bytecode: impl Into<Vec<u8>>) -> u64 {
        let rwasm_binary: Vec<u8> = bytecode.into();
        let import_linker = Runtime::new_linker();
        let result =
            Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker, true).unwrap();
        let circuit = FluentbaseCircuit::from_execution_result(&result);
        let degree: u32 = 17;
        let general_params = get_general_params(degree);
        let key = {
            let verifying_key =
                keygen_vk(&general_params, &circuit).expect("keygen_vk should not fail");
            let key = keygen_pk(&general_params, verifying_key, &circuit)
                .expect("keygen_pk should not fail");
            key.clone()
        };
        let elapsed = test_actual(circuit, vec![vec![Fr::zero()]], key, degree);
        println!("elapsed time (gen/proof/verify): {}ms", elapsed);
        elapsed
    }

    #[test]
    #[ignore]
    fn test_simple_proof() {
        gen_proof_verify(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
            .op_i32_const(3)
            .op_i32_add()
            .op_drop()
        ));
    }

    fn wasm2rwasm(wasm_binary: &[u8], import_linker: &ImportLinker) -> Vec<u8> {
        Compiler::new_with_linker(&wasm_binary.to_vec(), Some(import_linker))
            .unwrap()
            .finalize()
            .unwrap()
    }

    #[test]
    #[ignore]
    fn test_greeting() {
        let wasm_binary = include_bytes!("../../runtime/examples/bin/greeting.wasm");
        let import_linker = Runtime::new_linker();
        let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
        gen_proof_verify(rwasm_binary);
    }

    #[test]
    #[ignore]
    fn test_several_opcodes() {
        for iters in [100, 1000] {
            let mut bytecode = InstructionSet::new();
            (0..iters).for_each(|i| bytecode.op_i32_const(i));
            println!("proving {} iters", iters);
            let elapsed_time_ms = gen_proof_verify(bytecode);
            println!("est. ms per iter {}", elapsed_time_ms / iters as u64);
        }
    }
}
