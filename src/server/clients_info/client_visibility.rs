use bevy::{
    prelude::*,
    utils::{hashbrown::hash_map::Entry, EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

/// Entity visibility settings for a client.
pub struct ClientVisibility {
    filter: VisibilityFilter,

    /// Visibility for a specific entity that has been cached for re-referencing.
    ///
    /// Used as an optimization by server replication.
    cached_visibility: Visibility,
}

impl ClientVisibility {
    /// Creates a new instance based on the preconfigured policy.
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        match policy {
            VisibilityPolicy::All => Self::with_filter(VisibilityFilter::All {
                just_connected: true,
            }),
            VisibilityPolicy::Blacklist => Self::with_filter(VisibilityFilter::Blacklist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
            VisibilityPolicy::Whitelist => Self::with_filter(VisibilityFilter::Whitelist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
        }
    }

    /// Creates a new instance with a specific filter.
    fn with_filter(filter: VisibilityFilter) -> Self {
        Self {
            filter,
            cached_visibility: Default::default(),
        }
    }

    /// Resets the filter state to as it was after [`Self::new`].
    ///
    /// `cached_visibility` remains untouched.
    pub(super) fn clear(&mut self) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => *just_connected = true,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                list.clear();
                added.clear();
                removed.clear();
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                list.clear();
                added.clear();
                removed.clear();
            }
        }
    }

    /// Updates list information and its sets based on the filter.
    ///
    /// Should be called after each tick.
    pub(crate) fn update(&mut self) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => *just_connected = false,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                // Remove all entities queued for removal.
                for entity in removed.drain() {
                    list.remove(&entity);
                }
                added.clear();
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                // Change all recently added entities to `WhitelistInfo::Visible`
                // from `WhitelistInfo::JustVisible`.
                for entity in added.drain() {
                    list.insert(entity, WhitelistInfo::Visible);
                }
                removed.clear();
            }
        }
    }

    /// Removes a despawned entity tracked by this client.
    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        match &mut self.filter {
            VisibilityFilter::All { .. } => (),
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                if list.remove(&entity).is_some() {
                    added.remove(&entity);
                    removed.remove(&entity);
                }
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                if list.remove(&entity).is_some() {
                    added.remove(&entity);
                    removed.remove(&entity);
                }
            }
        }
    }

    /// Drains all entities for which visibility was lost during this tick.
    pub(super) fn drain_lost_visibility(&mut self) -> impl Iterator<Item = Entity> + '_ {
        match &mut self.filter {
            VisibilityFilter::All { .. } => VisibilityLostIter::AllVisible,
            VisibilityFilter::Blacklist { added, .. } => VisibilityLostIter::Lost(added.drain()),
            VisibilityFilter::Whitelist { removed, .. } => {
                VisibilityLostIter::Lost(removed.drain())
            }
        }
    }

    /// Sets visibility for a specific entity.
    ///
    /// Does nothing if the visibility policy for the server plugin is set to [`VisibilityPolicy::All`].
    pub fn set_visibility(&mut self, entity: Entity, visibile: bool) {
        match &mut self.filter {
            VisibilityFilter::All { .. } => {
                if visibile {
                    debug!(
                        "ignoring visibility enable due to {:?}",
                        VisibilityPolicy::All
                    );
                } else {
                    warn!(
                        "ignoring visibility disable due to {:?}",
                        VisibilityPolicy::All
                    );
                }
            }
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                if visibile {
                    if let Entry::Occupied(mut entry) = list.entry(entity) {
                        if !added.remove(&entity) {
                            removed.insert(entity);
                            // For blacklist we don't remove the entity right away.
                            // Instead we mark it as queued for removal and remove it
                            // later in `Self::update`. This allows us to avoid accessing
                            // the blacklist's `removed` field in `Self::cache_visibility`.
                            entry.insert(BlacklistInfo::QueuedForRemoval);
                        } else {
                            // If the entity was previously added in this tick, then do not consider it removed.
                            entry.remove();
                        }
                    }
                } else {
                    if *list.entry(entity).or_insert(BlacklistInfo::Hidden) == BlacklistInfo::Hidden
                    {
                        // Do not mark an entry as newly added if the entry was already in the list.
                        added.insert(entity);
                    }
                    removed.remove(&entity);
                }
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                if visibile {
                    // Similar to blacklist removal, we don't just add the entity to the list.
                    // Instead we mark it as `WhitelistInfo::JustAdded` and then set it to
                    // 'WhitelistInfo::Visible' in `Self::update`.
                    // This allows us to avoid accessing the whitelist's `added` field in
                    // `Self::cache_visibility`.
                    if *list.entry(entity).or_insert(WhitelistInfo::JustAdded)
                        == WhitelistInfo::JustAdded
                    {
                        // Do not mark an entry as newly added if the entry was already in the list.
                        added.insert(entity);
                    }
                    removed.remove(&entity);
                } else if list.remove(&entity).is_some() {
                    if !added.remove(&entity) {
                        // If the entity wasn't previously added in this tick, then consider it removed.
                        removed.insert(entity);
                    }
                }
            }
        }
    }

    /// Gets visibility for a specific entity.
    pub fn is_visible(&self, entity: Entity) -> bool {
        match self.get_visibility_state(entity) {
            Visibility::Hidden => false,
            Visibility::Gained | Visibility::Visible => true,
        }
    }

    /// Caches visibility for a specific entity.
    ///
    /// Can be obtained later from [`Self::cached_visibility`].
    pub(crate) fn cache_visibility(&mut self, entity: Entity) {
        self.cached_visibility = self.get_visibility_state(entity);
    }

    /// Returns visibility cached by the last call of [`Self::cache_visibility`].
    pub(crate) fn cached_visibility(&self) -> Visibility {
        self.cached_visibility
    }

    /// Returns visibility including
    fn get_visibility_state(&self, entity: Entity) -> Visibility {
        match &self.filter {
            VisibilityFilter::All { just_connected } => {
                if *just_connected {
                    Visibility::Gained
                } else {
                    Visibility::Visible
                }
            }
            VisibilityFilter::Blacklist { list, .. } => match list.get(&entity) {
                Some(BlacklistInfo::QueuedForRemoval) => Visibility::Gained,
                Some(BlacklistInfo::Hidden) => Visibility::Hidden,
                None => Visibility::Visible,
            },
            VisibilityFilter::Whitelist { list, .. } => match list.get(&entity) {
                Some(WhitelistInfo::JustAdded) => Visibility::Gained,
                Some(WhitelistInfo::Visible) => Visibility::Visible,
                None => Visibility::Hidden,
            },
        }
    }
}

/// Filter for [`ClientVisibility`] based on [`VisibilityPolicy`].
enum VisibilityFilter {
    All {
        /// Indicates that the client has just connected to the server.
        ///
        /// If true, then visibility of all entities has been gained.
        just_connected: bool,
    },
    Blacklist {
        /// All blacklisted entities and an indicator of whether they are in the queue for deletion
        /// at the end of this tick.
        list: EntityHashMap<Entity, BlacklistInfo>,

        /// All entities that were removed from the list in this tick.
        ///
        /// Visibility of these entities has been lost.
        added: EntityHashSet<Entity>,

        /// All entities that were added to the list in this tick.
        ///
        /// Visibility of these entities has been gained.
        removed: EntityHashSet<Entity>,
    },
    Whitelist {
        /// All whitelisted entities and an indicator of whether they were added to the list in
        /// this tick.
        list: EntityHashMap<Entity, WhitelistInfo>,

        /// All entities that were added to the list in tVisibility of these entities has been gained.his tick.
        ///
        /// Visibility of these entities has been gained.
        added: EntityHashSet<Entity>,

        /// All entities that were removed from the list in this tick.
        ///
        /// Visibility of these entities has been lost.
        removed: EntityHashSet<Entity>,
    },
}

#[derive(PartialEq, Clone, Copy)]
enum WhitelistInfo {
    Visible,
    JustAdded,
}

#[derive(PartialEq, Clone, Copy)]
enum BlacklistInfo {
    Hidden,
    QueuedForRemoval,
}

/// Visibility state for an entity in the current tick, from the perspective of one client.
///
/// Note that the distinction between 'lost visibility' and 'don't have visibility' is not exposed here.
/// There is only `Visibility::Hidden` to encompass both variants.
///
/// Lost visibility is handled separately with [`ClientVisibility::iter_lost_visibility`].
#[derive(PartialEq, Default, Clone, Copy)]
pub(crate) enum Visibility {
    /// The client does not have visibility of the entity in this tick.
    #[default]
    Hidden,
    /// The client gained visibility of the entity in this tick (it was not visible in the previous tick).
    Gained,
    /// The entity is visible to the client (and was visible in the previous tick).
    Visible,
}

enum VisibilityLostIter<T> {
    AllVisible,
    Lost(T),
}

impl<T: Iterator> Iterator for VisibilityLostIter<T> {
    type Item = T::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            VisibilityLostIter::AllVisible => None,
            VisibilityLostIter::Lost(entities) => entities.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::All);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(
            visibility.is_visible(Entity::PLACEHOLDER),
            "shouldn't have any effect for this policy"
        );
    }

    #[test]
    fn blacklist_insertion() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Blacklist);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_emtpy_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Blacklist);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Blacklist);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.update();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_insertion_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Blacklist);

        // Insert and remove from the list.
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Blacklist {
            list,
            added,
            removed,
        } = visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_insertion() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Whitelist);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a whitelist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_emtpy_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Whitelist);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = visibility.filter
        else {
            panic!("filter should be a whitelist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Whitelist);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.update();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a whitelist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = &visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_insertion_removal() {
        let mut visibility = ClientVisibility::new(VisibilityPolicy::Whitelist);

        // Insert and remove from the list.
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityFilter::Whitelist {
            list,
            added,
            removed,
        } = visibility.filter
        else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!added.contains(&Entity::PLACEHOLDER));
        assert!(!removed.contains(&Entity::PLACEHOLDER));
    }
}
