#![forbid(unsafe_code)]

use ragavan_core::{Port, WorktreeIdentity};

const PORT_RANGE_START: u16 = 10_000;
const PORT_RANGE_SIZE: u64 = 20_000;

pub fn port_for(identity: &WorktreeIdentity) -> Port {
    let repository_slot = stable_hash(identity.repository_id()) % PORT_RANGE_SIZE;
    let worktree_slot = stable_hash(identity.worktree_id()) % PORT_RANGE_SIZE;
    let value = PORT_RANGE_START + ((repository_slot + worktree_slot) % PORT_RANGE_SIZE) as u16;

    Port::new(value).expect("Ragavan's port range excludes zero")
}

fn stable_hash(value: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
