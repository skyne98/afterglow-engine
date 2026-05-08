# Bevy 0.18 Animation System — Deep Dive

## Architecture

Bevy 0.18 has a completely redesigned animation system (RFC 51) based on an **animation graph** rather than a simple player. Key difference from pre-0.18:

| Feature | Old (pre-0.18) | New (0.18) |
|---|---|---|
| Architecture | Single clip per player | **Player + Graph DAG** |
| Multiple animations | One at a time | Blend tree of clips |
| Blending | None | Blend nodes + Add nodes, weight control |
| Masks | None | 64-bit bone mask groups |
| Custom properties | Transform only | `AnimatableProperty` trait, any component |
| Targets | Hardcoded | `AnimationTargetId` (UUID from bone path) |
| Retargeting | Not possible | Built-in (same bone names → same IDs) |
| Graph serialization | N/A | `.animgraph.ron` files |
| Events | None | Timeline events with observers |
| Transitions | Manual | `AnimationTransitions` with crossfade |
| Evaluation | Direct clip | Stack-based graph traversal |
| Threading | None | Parallel target evaluation |

## Core Types

### `AnimationClip` (Asset)
Stores curves keyed by `AnimationTargetId` → `Vec<VariableCurve>`:
```rust
pub struct AnimationClip {
    curves: HashMap<AnimationTargetId, Vec<VariableCurve>>,
    events: HashMap<AnimationEventTarget, Vec<TimedAnimationEvent>>,
    duration: f32,
}
```
Methods: `add_curve_to_target(id, curve)`, `add_event(time, event)`, `add_event_to_target(id, time, event)`.

### `AnimationPlayer` (Component)
Manages a set of `ActiveAnimation`s keyed by `AnimationNodeIndex`:
```rust
pub struct AnimationPlayer {
    active_animations: HashMap<AnimationNodeIndex, ActiveAnimation>,
}
```
Key methods: `start(anim)`, `play(anim)`, `stop(anim)`, `stop_all()`, `pause_all()`, `adjust_speeds(factor)`, `seek_all_by(amount)`.

### `AnimationGraph` (Asset)
A `petgraph` DAG of `AnimationGraphNode`s:
```rust
pub struct AnimationGraph {
    pub graph: DiGraph<AnimationGraphNode, (), u32>,
    pub root: NodeIndex,
    pub mask_groups: HashMap<AnimationTargetId, AnimationMask>,
}
```

`AnimationGraphNode { node_type, mask: u64, weight: f32 }` where `node_type` is:
- `Clip(Handle<AnimationClip>)` — leaf node
- `Blend` — blends children (weights normalized)
- `Add` — additive blend (weights NOT normalized)

Construction: `AnimationGraph::new()` / `from_clip(clip)` / `from_clips([...])`.
Add: `add_clip(handle, weight, parent)`, `add_blend(weight, parent)`, `add_additive_blend(weight, parent)`.

### `AnimationGraphHandle` (Component)
`Handle<AnimationGraph>` placed on the same entity as `AnimationPlayer`.

### `AnimationTargetId`
UUID derived from bone path (e.g., hash of `["Arm", "Hand"]`). Any armature with the same bone names produces the same IDs → retargeting works automatically.

### `ActiveAnimation`
Per-animation state: `weight`, `repeat` (`Never`/`Count(u32)`/`Forever`), `speed`, `elapsed`, `seek_time`, `paused`.

## Graph Evaluation (Stack-Based)

1. **Postorder traversal** of the graph (via `ThreadedAnimationGraph`)
2. **Clip nodes**: sample curve → push `(value, weight, node_index)` onto evaluator stack
3. **Blend nodes**: `evaluator.blend(node)` + `push_blend_register(weight)` → interpolated result
4. **Add nodes**: same but `add()` — additive combination
5. **Root commit**: `evaluator.commit(entity)` → write final value to component

## AnimationCurve & Animatable

```rust
pub trait AnimationCurve: Debug + Send + Sync + 'static {
    fn domain(&self) -> Interval;
    fn evaluator_id(&self) -> EvaluatorId<'_>;
    fn create_evaluator(&self) -> Box<dyn AnimationCurveEvaluator>;
    fn apply(&self, evaluator: &mut dyn AnimationCurveEvaluator, t: f32, weight: f32, node: AnimationNodeIndex) -> Result;
}
```

```rust
pub trait Animatable: Reflect + Sized + Send + Sync + 'static {
    fn interpolate(a: &Self, b: &Self, time: f32) -> Self;
    fn blend(inputs: impl Iterator<Item = BlendInput<Self>>) -> Self;
}
```

Implemented for: `f32`, `Vec2/3/4`, `Quat`, `Transform`, `bool`, colors.

`AnimatableCurve<P, C>` wraps a curve + property selector together:
```rust
AnimatableCurve::new(
    animated_field!(Transform::translation),
    UnevenSampleAutoCurve::new(keyframes).unwrap(),
)
```

## Animation Transitions (Crossfade)

```rust
pub struct AnimationTransitions {
    main_animation: Option<AnimationNodeIndex>,
    transitions: Vec<AnimationTransition>,  // fading-out animations
}
```

`transitions.play(&mut player, new_anim, duration)` fades out the old and fades in the new.

## Animation Events

Custom event types with `#[derive(AnimationEvent)]`:
```rust
#[derive(AnimationEvent, Clone)]
struct MyEvent { value: String }

clip.add_event(0.5, MyEvent { value: "hello".into() });
```

Events fire on targets via observers:
```rust
commands.observe(|trigger: On<MyEvent>, ...| { ... });
```

## Skinning / Skeletal Animation

```rust
pub struct SkinnedMesh {
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    pub joints: Vec<Entity>,
}
```

glTF import: creates joint hierarchy → `AnimationPlayer` on root → `AnimationTargetId` + `AnimatedBy` on each bone → `SkinnedMesh` with joint references.

Vertex attributes: `ATTRIBUTE_JOINT_INDEX` (4 x u16), `ATTRIBUTE_JOINT_WEIGHT` (4 x f32).

## Morph Targets

`MorphWeights` component + `WeightsCurve<C>` for animation.

## Interpolation Types

- `UnevenSampleAutoCurve<T>` — linear (via `Animatable::interpolate`)
- `SteppedKeyframeCurve<T>` — step
- `CubicKeyframeCurve<V>` — cubic spline (VectorSpace types)
- `CubicRotationCurve` — cubic spline for Quat (normalized)
- `WideLinear/Stepped/CubicKeyframeCurve` — for morph targets (all weights at once)

## System Order

```
PostUpdate:
  thread_animation_graphs
  → advance_transitions
  → advance_animations (parallel)
  → animate_targets (parallel, before InheritWeight, before TransformSystems::Propagate)
  → trigger_untargeted_animation_events
  → expire_completed_transitions
```

## Quickstart

```rust
let mut clip = AnimationClip::default();
clip.add_curve_to_target(AnimationTargetId::from(&Name::new("MyName")),
    AnimatableCurve::new(animated_field!(Transform::translation),
        UnevenSampleAutoCurve::new([(0.0, Vec3::ZERO), (2.0, Vec3::X * 5.0)]).unwrap()));

let (graph, node_idx) = AnimationGraph::from_clip(animations.add(clip));
let mut player = AnimationPlayer::default();
player.play(node_idx).repeat();

commands.spawn((Mesh3d(..), MeshMaterial3d(..), Name::new("MyName"),
    AnimationTargetId::from(&Name::new("MyName")), AnimatedBy(entity),
    AnimationGraphHandle(graphs.add(graph)), player));
```

## Root Motion

Not built-in. Must be implemented manually by reading root bone transform deltas.

## References

- Examples: `examples/animation/` (animated_transform, animation_graph, animation_masks, morph_targets, custom_skinned_mesh, etc.)
- `bevy_animation` crate: `lib.rs`, `graph.rs`, `transition.rs`, `animation_event.rs`, `morph.rs`, `animation_curves.rs`, `gltf_curves.rs`
- RFC 51: https://github.com/bevyengine/rfcs/blob/main/rfcs/51-animation-composition.md
