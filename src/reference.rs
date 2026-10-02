//! Static evidence fixtures, not observations from a running XP screensaver.
//! Kept behind cfg(test); ordinary builds never read private reference artifacts.

pub fn rng_rows() -> Vec<(u32, usize, u32, u32)> {
    let text = include_str!("../tests/fixtures/xp-rand-v1.tsv");
    let mut lines = text.lines();
    for expected in [
        "schema\txp-rand-v1",
        "sha256\t664b4b1da036eb36b13a3bce2fd572e37127223cbf6796adde12cbfb3af0d893",
        "evidence\tstatic-assembly",
        "warmup\t10",
        "seed\tdraw\tstate\tvalue",
    ] {
        assert_eq!(
            lines.next(),
            Some(expected),
            "fixture identity/schema mismatch"
        );
    }
    lines
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 4, "invalid reference row: {line}");
            (
                fields[0].parse().unwrap(),
                fields[1].parse().unwrap(),
                u32::from_str_radix(fields[2], 16).unwrap(),
                fields[3].parse().unwrap(),
            )
        })
        .collect()
}

#[test]
fn static_fixture_has_checked_identity_and_ordered_rows() {
    let rows = rng_rows();
    assert_eq!(rows.len(), 48);
    for (group, seed) in rows.as_chunks::<12>().0.iter().zip([0, 1, 42, u32::MAX]) {
        for (index, &(actual_seed, draw, state, value)) in group.iter().enumerate() {
            assert_eq!(actual_seed, seed);
            assert_eq!(draw, index);
            assert_eq!(value, (state >> 16) & 0x7fff);
        }
    }
}
