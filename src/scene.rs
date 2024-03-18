use bevy::{ecs::entity::EntityHashMap, prelude::*, scene::DynamicEntity};

use crate::server::replicated_archetypes::ReplicatedArchetypes;

/**
Fills scene with all replicated entities and their components.

Entities won't have the [`Replication`](crate::core::component_rules::Replication) component.
So on deserialization you need to insert it back if you want entities to continue to replicate.

Internally relies on [`ReplicatedArchetypes`] which updates in
[`ServerSet::UpdateArchetypes`](crate::server::ServerSet::UpdateArchetypes).
So it's preferred to replicate into scene after this set.

# Panics

Panics if any replicated component is not registered using [`App::register_type`]
or `#[reflect(Component)]` is missing.

# Examples

```
use bevy::{prelude::*, asset::ron, scene::serde::SceneDeserializer};
use bevy_replicon::{server::replicated_archetypes::ReplicatedArchetypes, prelude::*, scene};
use serde::de::DeserializeSeed;
# let mut world = World::new();
# world.init_resource::<AppTypeRegistry>();
# world.init_resource::<ReplicatedArchetypes>();

// Serialization
let registry = world.resource::<AppTypeRegistry>();
let mut scene = DynamicScene::default();
scene::replicate_into(&mut scene, &world);
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

// All saved entities should have `Replication` component.
for entity in &mut scene.entities {
    entity.components.push(Replication.clone_value());
}
```
*/
pub fn replicate_into(scene: &mut DynamicScene, world: &World) {
    let entities_iter = scene
        .entities
        .drain(..)
        .map(|dyn_entity| (dyn_entity.entity, dyn_entity.components));
    let mut entities = EntityHashMap::from_iter(entities_iter);

    let registry = world.resource::<AppTypeRegistry>();
    let registry = registry.read();
    let replicated_archetypes = world.resource::<ReplicatedArchetypes>();
    for replicated_archetype in replicated_archetypes.iter() {
        // SAFETY: all IDs from replicated archetypes obtained from real archetypes.
        let archetype = unsafe {
            world
                .archetypes()
                .get(replicated_archetype.id())
                .unwrap_unchecked()
        };

        // Populate entities ahead of time in order to extract entities without components too.
        for entity in archetype.entities() {
            entities.entry(entity.id()).or_default();
        }

        for replicated_component in replicated_archetype.components() {
            // SAFETY: `component_id` obtained from the replicated component.
            let component_info = unsafe {
                world
                    .components()
                    .get_info_unchecked(replicated_component.component_id)
            };
            let type_name = component_info.name();
            let type_id = component_info
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

    let dyn_entities_iter = entities
        .drain()
        .map(|(entity, components)| DynamicEntity { entity, components });
    scene.entities.extend(dyn_entities_iter);
}
