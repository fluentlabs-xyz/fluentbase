extern crate alloc;

use crate::jwks::{JWK, JWKS};
use alloc::vec;

/// Google OAuth2 JWKS Keys(Hardcoded for now)
///
pub fn google_jwks() -> JWKS {
    JWKS {
        keys: vec![
            // Real Google JWKS Key 1
            // Fetched from: https://www.googleapis.com/oauth2/v3/certs
            // Updated: October 13, 2025
            JWK {
                kty: "RSA".into(),
                alg: Some("RS256".into()),
                kid: Some("c8ab71530972bba20b49f78a09c9852c43ff9118".into()),
                key_use: Some("sig".into()),
                n: Some("vG5pJE-wQNbH7tvZU3IgjdeHugdw2x5eXPe47vOP3dIy4d9HnCWSTroJLtPYA1SFkcl8FlgrgWspCGBzJ8gwMo81Tk-5hX2pWXsNKrOH8R01jFqIn_UBwhmqU-YDde1R4w9upLzwNyl9Je_VY65EKrMOZG9u4UYtzTkNFLf1taBe0gIM20VSAcClUhTGpE3MX9lXxQqN3Hoybja7C_SZ8ymcnB5h-20ynZGgQybZRU43KcZkIMK2YKkLd7Tn4UQeSRPbmlbm5a0zbs5GpcYB7MONYh7MD16FTS72-tYKX-kDh3NltO6HQsV9pfoOi7qJrFaYWP3AHd_h7mWTHIkNjQ".into()),
                e: Some("AQAB".into()),
                crv: None,
                x: None,
                y: None,
            },
            // Real Google JWKS Key 2
            JWK {
                kty: "RSA".into(),
                alg: Some("RS256".into()),
                kid: Some("fb9f9371d5755f3e383a40ab3a172cd8baca517f".into()),
                key_use: Some("sig".into()),
                n: Some("to2hcsFNHKquhCdUzXWdP8yxnGqxFWJlRT7sntBgp47HwxB9HFc-U_AB1JT8xe1hwDpWTheckoOfpLgo7_ROEsKpVJ_OXnotL_dgNwbprr-T_EFJV7qOEdHL0KmrnN-kFNLUUSqSChPYVh1aEjlPfXg92Yieaaz2AMMtiageZrKoYnrGC0z4yPNYFj21hO1x6mvGIjmpo6_fe91o-buZNzzkmYlGsFxdvUxYAvgk-5-7D10UTTLGh8bUv_BQT3aRFiVRS5d07dyCJ4wowzxYlPSM6lnfUlvHTWyPL4JysMGeu-tbPA-5QvwCdSGpfWFQbgMq9NznBtWb99r1UStpBQ".into()),
                e: Some("AQAB".into()),
                crv: None,
                x: None,
                y: None,
            },
        ],
    }
}
