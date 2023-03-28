use rand::Rng;

use crate::bip39_words::WORDS;

pub fn generate_bip39_mnemonic() -> String {
    let mut rng = rand::thread_rng();
    let mut chosen_words = Vec::with_capacity(24);
    for _ in 0..24 {
        let ix = rng.gen_range(0..WORDS.len());
        chosen_words.push(WORDS[ix]);
    }

    let mnemonic = chosen_words.join(" ");
    mnemonic
}
