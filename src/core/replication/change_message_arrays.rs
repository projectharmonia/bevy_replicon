use bitflags::bitflags;

bitflags! {
    /// Types of arrays included in the change message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    pub(crate) struct ChangeMessageArrays: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}

impl ChangeMessageArrays {
    /// Returns the last array in the message for which the bit is set.
    pub(crate) fn last(self) -> ChangeMessageArrays {
        debug_assert!(!self.is_empty());
        let zeroes = u8::BITS - 1 - self.bits().leading_zeros();
        ChangeMessageArrays::from_bits_retain(1 << zeroes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last() {
        assert_eq!(
            ChangeMessageArrays::CHANGES.last(),
            ChangeMessageArrays::CHANGES
        );
        assert_eq!(
            ChangeMessageArrays::MAPPINGS.last(),
            ChangeMessageArrays::MAPPINGS
        );
        assert_eq!(
            ChangeMessageArrays::all().last(),
            ChangeMessageArrays::CHANGES
        );
        assert_eq!(
            (ChangeMessageArrays::DESPAWNS | ChangeMessageArrays::REMOVALS).last(),
            ChangeMessageArrays::REMOVALS
        );
    }
}
