//! Cross-contract invocation payload parser/executor: any JSON campaign must
//! be rejected or executed through `Result`, never by panicking.
#![no_main]

use invocation_fuzzer::{execute_campaign, validate_campaign, Campaign};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(campaign) = serde_json::from_slice::<Campaign>(data) else {
        return;
    };
    if validate_campaign(&campaign).is_ok() {
        let _ = execute_campaign(&campaign);
    }
});
