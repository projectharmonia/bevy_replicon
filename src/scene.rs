use bevy::{ecs::entity::EntityHashMap, prelude::*, scene::DynamicEntity};

use crate::{core::replication_rules::ReplicationRules, Replication};

/**
Fills scene with all replicated entities and their components.

Entities won't have the [`Replication`] component.
So on deserialization you need to insert it back if you want entities to continue to replicate.

# Panics

Panics if any replicated component is not registered using [`App::register_type`]
or `#[reflect(Component)]` is missing.

# Examples

```
use bevy::{prelude::*, asset::ron, scene::serde::SceneDeserializer};
use bevy_replicon::{prelude::*, scene};
use serde::de::DeserializeSeed;
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);

// Serialization
let registry = app.world.resource::<AppTypeRegistry>();
let mut scene = DynamicScene::default();
scene::replicate_into(&mut scene, &app.world);
let scene = scene
    .serialize_ron(&registry)
    .expect("scene should be serialized");

// Deserialization
let scene_deserializer = SceneDeserializer {
    type_registry: &registry.read(),
};
let mut deserializer =
    ron::Deserializer::from_str(&scene).expect("scene should be serialized as valid ron");
let mut scene = scene_deserializer
    .deserialize(&mut deserializer)
    .expect("ron should be convertible to scene");

// Re-insert `Replication` component.
for entity in &mut scene.entities {
    entity.components.push(Replication.clone_value());
}
```
*/
pub fn replicate_into(scene: &mut DynamicScene, world: &World) {
    let Some(marker_id) = world.component_id::<Replication>() else {
        // Components are initialized lazily.
        // If there is no replication marker, then we have nothing to replicate.
        return;
    };

    let entities_iter = scene
        .entities
        .drain(..)
        .map(|dyn_entity| (dyn_entity.entity, dyn_entity.components));
    let mut entities = EntityHashMap::from_iter(entities_iter);

    let registry = world.resource::<AppTypeRegistry>();
    let rules = world.resource::<ReplicationRules>();
    let registry = registry.read();
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.contains(marker_id))
    {
        // Populate entities ahead of time in order to extract entities without components too.
        for entity in archetype.entities() {
            entities.entry(entity.id()).or_default();
        }

        for rule in rules.iter().filter(|rule| rule.matches(archetype)) {
            for component_id in rule
                .components()
                .iter()
                .map(|fns_info| fns_info.component_id())
            {
                // SAFETY: replication rules can be registered only with valid component IDs.
                let replicated_component =
                    unsafe { world.components().get_info_unchecked(component_id) };
                let type_name = replicated_component.name();
                let type_id = replicated_component
                    .type_id()
                    .unwrap_or_else(|| panic!("{type_name} should have registered TypeId"));
                let registration = registry
                    .get(type_id)
                    .unwrap_or_else(|| panic!("{type_name} should be registered"));
                let reflect_component = registration
                    .data::<ReflectComponent>()
                    .unwrap_or_else(|| panic!("{type_name} should have reflect(Component)"));

                for entity in archetype.entities() {
                    let component = reflect_component
                        .reflect(world.entity(entity.id()))
                        .unwrap_or_else(|| panic!("entity should have {type_name}"));

                    let components = entities
                        .get_mut(&entity.id())
                        .expect("all entities should be populated ahead of time");
                    components.push(component.clone_value());
                }
            }
        }
    }

    let dyn_entities_iter = entities
        .drain()
        .map(|(entity, components)| DynamicEntity { entity, components });
    scene.entities.extend(dyn_entities_iter);
}
