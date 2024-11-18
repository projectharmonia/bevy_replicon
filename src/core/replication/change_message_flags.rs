use bitflags::bitflags;

bitflags! {
    /// Types of data included in the change message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    pub(crate) struct ChangeMessageFlags: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}

impl ChangeMessageFlags {
    /// Returns the last set flag in the message.
    pub(crate) fn last(self) -> ChangeMessageFlags {
        debug_assert!(!self.is_empty());
        let zeroes = u8::BITS - 1 - self.bits().leading_zeros();
        ChangeMessageFlags::from_bits_retain(1 << zeroes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last() {
        assert_eq!(
            ChangeMessageFlags::CHANGES.last(),
            ChangeMessageFlags::CHANGES
        );
        assert_eq!(
            ChangeMessageFlags::MAPPINGS.last(),
            ChangeMessageFlags::MAPPINGS
        );
        assert_eq!(
            ChangeMessageFlags::all().last(),
            ChangeMessageFlags::CHANGES
        );
        assert_eq!(
            (ChangeMessageFlags::DESPAWNS | ChangeMessageFlags::REMOVALS).last(),
            ChangeMessageFlags::REMOVALS
        );
    }
}
