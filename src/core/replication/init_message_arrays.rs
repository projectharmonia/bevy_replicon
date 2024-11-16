use bitflags::bitflags;

bitflags! {
    /// Types of arrays included in the init message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    pub(crate) struct InitMessageArrays: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}

impl InitMessageArrays {
    /// Returns the last array in the message for which the bit is set.
    pub(crate) fn last(self) -> InitMessageArrays {
        debug_assert!(!self.is_empty());
        let zeroes = u8::BITS - 1 - self.bits().leading_zeros();
        InitMessageArrays::from_bits_retain(1 << zeroes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last() {
        assert_eq!(
            InitMessageArrays::CHANGES.last(),
            InitMessageArrays::CHANGES
        );
        assert_eq!(
            InitMessageArrays::MAPPINGS.last(),
            InitMessageArrays::MAPPINGS
        );
        assert_eq!(InitMessageArrays::all().last(), InitMessageArrays::CHANGES);
        assert_eq!(
            (InitMessageArrays::DESPAWNS | InitMessageArrays::REMOVALS).last(),
            InitMessageArrays::REMOVALS
        );
    }
}
