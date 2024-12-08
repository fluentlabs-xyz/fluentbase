use fluentbase_sdk::SovereignAPI;
use phantom_type::PhantomType;
use solana_program::{
    hash::{Hash, Hasher},
    keccak,
};

pub trait HasherImpl {
    const NAME: &'static str;
    type Output: AsRef<[u8]>;

    fn create_hasher() -> Self;
    fn hash(&mut self, val: &[u8]);
    fn result(self) -> Self::Output;
}

pub struct Sha256Hasher(Hasher);
impl HasherImpl for Sha256Hasher {
    const NAME: &'static str = "Sha256";
    type Output = Hash;

    fn create_hasher() -> Self {
        Sha256Hasher(Hasher::default())
    }

    fn hash(&mut self, val: &[u8]) {
        self.0.hash(val);
    }

    fn result(self) -> Self::Output {
        self.0.result()
    }
}

// pub struct Keccak256Hasher(keccak::Hasher);
//
// impl HasherImpl for Keccak256Hasher {
//     const NAME: &'static str = "Keccak256";
//     type Output = keccak::Hash;
//
//     fn create_hasher() -> Self {
//         Keccak256Hasher(keccak::Hasher::default())
//     }
//
//     fn hash(&mut self, val: &[u8]) {
//         self.0.hash(val);
//     }
//
//     fn result(self) -> Self::Output {
//         self.0.result()
//     }
// }

pub struct Keccak256Hasher<SDK> {
    initiated: bool,
    value: [u8; 32],
    _sdk: PhantomType<SDK>,
}
impl<SDK: SovereignAPI> HasherImpl for Keccak256Hasher<SDK> {
    const NAME: &'static str = "Keccak256";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        Keccak256Hasher {
            initiated: false,
            value: Default::default(),
            _sdk: Default::default(),
        }
    }

    fn hash(&mut self, val: &[u8]) {
        if self.initiated {
            panic!("accumulation not supported yet")
        } else {
            self.value = SDK::keccak256(val).0;
            self.initiated = true;
        }
    }

    fn result(self) -> Self::Output {
        self.value
    }
}

pub struct PoseidonHasher<SDK> {
    initiated: bool,
    value: [u8; 32],
    _sdk: PhantomType<SDK>,
}
impl<SDK: SovereignAPI> HasherImpl for PoseidonHasher<SDK> {
    const NAME: &'static str = "Poseidon";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        PoseidonHasher {
            initiated: false,
            value: Default::default(),
            _sdk: Default::default(),
        }
    }

    fn hash(&mut self, val: &[u8]) {
        if self.initiated {
            panic!("accumulation not supported yet")
        } else {
            self.value = SDK::poseidon(val).0;
            self.initiated = true;
        }
    }

    fn result(self) -> Self::Output {
        self.value
    }
}
