//! Generated agent names, `<adjective>-<bird>` (brief §3). Choosing indices needs randomness, which lives in
//! `corvus-core`; this module only holds the words and builds names.

use crate::name::AgentName;

pub const ADJECTIVES: &[&str] = &[
    "able", "agile", "amber", "ample", "azure", "bold", "brave", "bright", "brisk", "calm", "candid",
    "civil", "clear", "clever", "cosmic", "crisp", "curious", "daring", "dawn", "deft", "eager", "early",
    "earnest", "easy", "fair", "fast", "fierce", "fleet", "fond", "frank", "fresh", "gentle", "glad",
    "golden", "grand", "hardy", "hasty", "hearty", "honest", "humble", "jolly", "keen", "kind", "lively",
    "loyal", "lucid", "lucky", "merry", "mild", "misty", "modest", "nimble", "noble", "patient", "plucky",
    "polite", "proud", "quick", "quiet", "rapid", "ready", "regal", "robust", "rosy", "rustic", "sage",
    "serene", "sharp", "shy", "silent", "silver", "sleek", "smart", "snowy", "solar", "solid", "steady",
    "stout", "sunny", "swift", "tidy", "tranquil", "true", "trusty", "vivid", "warm", "wise", "witty",
    "young", "zesty",
];

pub const BIRDS: &[&str] = &[
    "albatross",
    "auk",
    "avocet",
    "bittern",
    "bluebird",
    "bunting",
    "buzzard",
    "chickadee",
    "condor",
    "cormorant",
    "crane",
    "crow",
    "cuckoo",
    "curlew",
    "dipper",
    "dove",
    "dunlin",
    "eagle",
    "egret",
    "eider",
    "falcon",
    "finch",
    "flicker",
    "gannet",
    "godwit",
    "goldfinch",
    "goose",
    "goshawk",
    "grebe",
    "grouse",
    "gull",
    "harrier",
    "hawk",
    "heron",
    "hoopoe",
    "ibis",
    "jackdaw",
    "jay",
    "kestrel",
    "killdeer",
    "kingfisher",
    "kite",
    "kiwi",
    "lapwing",
    "lark",
    "linnet",
    "loon",
    "magpie",
    "mallard",
    "martin",
    "merlin",
    "nighthawk",
    "nightjar",
    "nuthatch",
    "oriole",
    "osprey",
    "owl",
    "parrot",
    "pelican",
    "peregrine",
    "petrel",
    "pheasant",
    "pipit",
    "plover",
    "puffin",
    "quail",
    "rail",
    "raven",
    "redstart",
    "robin",
    "rook",
    "sandpiper",
    "shrike",
    "siskin",
    "skylark",
    "snipe",
    "sparrow",
    "starling",
    "stork",
    "swallow",
    "swift",
    "tanager",
    "tern",
    "thrush",
    "titmouse",
    "toucan",
    "turnstile",
    "vireo",
    "warbler",
    "waxwing",
    "wigeon",
    "wren",
    "yellowlegs",
];

/// `ADJECTIVES[a % len]-BIRDS[b % len]`.
pub fn generated_name(a: usize, b: usize) -> AgentName {
    let s = format!("{}-{}", ADJECTIVES[a % ADJECTIVES.len()], BIRDS[b % BIRDS.len()]);
    AgentName::parse(&s).expect("word lists only contain valid name characters")
}

/// `base-n` for the numeric-suffix fallback after many collisions (`n >= 2`).
pub fn suffixed_name(base: &AgentName, n: u32) -> AgentName {
    AgentName::parse(&format!("{base}-{n}")).expect("a generated base plus a number is a valid name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_combination_is_a_valid_name() {
        for a in 0..ADJECTIVES.len() {
            for b in 0..BIRDS.len() {
                let n = generated_name(a, b);
                assert!(n.as_str().len() <= 40);
                assert!(!n.as_str().contains(':'));
            }
        }
        assert_eq!(
            suffixed_name(&generated_name(0, 0), 12).as_str(),
            "able-albatross-12"
        );
    }

    #[test]
    fn words_are_unique_lowercase_ascii() {
        for list in [ADJECTIVES, BIRDS] {
            let mut v = list.to_vec();
            v.sort_unstable();
            v.dedup();
            assert_eq!(v.len(), list.len());
            assert!(list.iter().all(|w| w.chars().all(|c| c.is_ascii_lowercase())));
        }
    }

    #[test]
    fn indices_wrap() {
        assert_eq!(
            generated_name(ADJECTIVES.len(), BIRDS.len() + 1),
            generated_name(0, 1)
        );
    }
}
