#![allow(clippy::all, clippy::pedantic)]
pub mod com {
    pub mod daml {
        pub mod daml_lf_2 {
            #[cfg(not(feature = "no-generated"))]
            include!(concat!(env!("OUT_DIR"), "/daml_lf_2.rs"));
        }
        pub mod daml_lf {
            #[cfg(not(feature = "no-generated"))]
            include!(concat!(env!("OUT_DIR"), "/daml_lf.rs"));
        }
    }
}
