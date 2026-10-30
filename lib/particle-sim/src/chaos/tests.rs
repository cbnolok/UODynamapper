//! Rule-by-rule tests of the Chaos compiler and runtime, on small scripts.

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use super::compile::{ChaosCompileReport, compile_chaos_source};
use super::runtime::cel_uv;
use super::*;
use crate::math::Vec3;
use crate::render::{RenderFrame, SpriteKind, UvRect};
use crate::rng::Mt19937;
use crate::system::{NodeTransform, ParticleSystemInstance};

const EPSILON: f32 = 1e-4;
const SEED: u32 = 1234;
/// A frame step whose seconds are exact in f32 (250 * 0.001 rounds to 0.25).
const STEP_MS: u32 = 250;
const STEP_S: f32 = 0.25;
/// A one-second frame step.
const SECOND_MS: u32 = 1000;

fn script(body: &str) -> String {
    format!("particlesystem \"test\" 1\n{{\n{body}\n}}\n")
}

fn one_sequence(body: &str) -> String {
    script(&format!("eventsequence \"s\"\n{{\n{body}\n}}"))
}

fn compile(text: &str) -> ChaosCompileReport {
    compile_chaos_source(text.as_bytes())
}

fn desc(text: &str) -> ChaosSystemDesc {
    let report = compile(text);
    let errors: Vec<String> = report.errors().map(ToString::to_string).collect();
    report.desc.unwrap_or_else(|| panic!("compile errors: {errors:?}"))
}

fn error_kinds(text: &str) -> Vec<ChaosIssueKind> {
    let report = compile(text);
    assert!(report.desc.is_none(), "expected a compile error");
    report.issues.into_iter().filter(ChaosIssue::is_error).map(|issue| issue.kind).collect()
}

fn instance(text: &str) -> ChaosEffectInstance {
    let mut rng = Mt19937::new(SEED);
    let system = Arc::new(ChaosSystem::prepare(desc(text), &mut rng));
    ChaosEffectInstance::with_seed(system, SEED)
}

/// Starts `instance` and runs `updates` updates of `dt_ms` at a fixed node.
fn run(instance: &mut ChaosEffectInstance, looping: bool, duration_s: f32, updates: usize, dt_ms: u32, node: &NodeTransform) {
    instance.start(duration_s, looping);
    for _ in 0..updates {
        instance.update(dt_ms, node);
    }
}

fn particles(instance: &ChaosEffectInstance) -> &[runtime::ChaosParticle] {
    instance.emitter().particles(0)
}

// --- Compiler.

#[test]
fn multipliers_scale_later_events() {
    let desc = desc(&one_sequence(
        "type = point\nscaleMultiplier = 2\nvelocityMultiplier = random(1, 3)\n\
         initial scale = 1.5\ninitial velocity = XYZ(1, 2, 3)\ninitial length = 4",
    ));
    let events = &desc.sequences[0].events;
    assert_eq!(events[0].value.min[0], 3.0);
    // The multiplier is the range max.
    assert_eq!(&events[1].value.max[..3], &[3.0, 6.0, 9.0]);
    assert_eq!(events[2].value.min[0], 4.0);
    assert!(desc.sequences[0].flags.contains(ChaosSequenceFlags::SCALE_MULTIPLIER));
}

#[test]
fn rotation_is_converted_to_radians() {
    let desc = desc(&one_sequence("type = point\ninitial rotation = XYZ(0, 90, -180)"));
    let value = desc.sequences[0].events[0].value;
    assert!((value.min[1] - FRAC_PI_2).abs() < EPSILON);
    assert!((value.min[2] + std::f32::consts::PI).abs() < EPSILON);
    assert!(desc.sequences[0].flags.contains(ChaosSequenceFlags::ROTATION));
}

#[test]
fn property_after_event_is_an_error() {
    let kinds = error_kinds(&one_sequence("initial scale = 1\nlifetime = 2"));
    assert!(matches!(kinds.as_slice(), [ChaosIssueKind::PropertyAfterEvent { .. }]), "{kinds:?}");
}

#[test]
fn keywords_match_by_substring() {
    let report = compile(&one_sequence("type = point\nsorted = true\ninitial alphaColor = 0.5"));
    let desc = report.desc.as_ref().unwrap();
    assert!(desc.sequences[0].flags.contains(ChaosSequenceFlags::SORT));
    assert_eq!(desc.sequences[0].events[0].kind, ChaosEventKind::Alpha);
    assert!(report.issues.iter().any(|issue| matches!(
        &issue.kind,
        ChaosIssueKind::SubstringKeyword { written, keyword: "SORT" } if written == "sorted"
    )));
}

#[test]
fn plain_alpha_is_no_event() {
    let kinds = error_kinds(&one_sequence("initial alpha = 0.5"));
    assert!(matches!(kinds.as_slice(), [ChaosIssueKind::ExpectedEventProperty { .. }]), "{kinds:?}");
}

#[test]
fn emit_radius_has_no_handler() {
    let kinds = error_kinds(&one_sequence("emitRadius = XYZ(1, 1, 1)"));
    assert!(matches!(kinds.as_slice(), [ChaosIssueKind::UnknownVectorProperty { .. }]), "{kinds:?}");
}

#[test]
fn final_takes_the_lifetime_max_parsed_so_far() {
    let report = compile(&one_sequence("type = point\nfinal scale = 1"));
    assert_eq!(report.desc.as_ref().unwrap().sequences[0].events[0].time, Range::ZERO);
    assert!(report.issues.iter().any(|issue| issue.kind == ChaosIssueKind::FinalBeforeLifetime));

    let desc = desc(&one_sequence("type = point\nlifetime = random(1, 3)\nfinal scale = 1"));
    assert_eq!(desc.sequences[0].events[0].time, Range::constant(3.0));
}

#[test]
fn conflicting_flags_are_errors() {
    let local_then_stay = error_kinds(&one_sequence("local = true\nstayAtEmitter = true"));
    assert_eq!(local_then_stay, [ChaosIssueKind::StayAtEmitterWithLocal]);
    let stay_then_local = error_kinds(&one_sequence("stayAtEmitter = 1\nlocal = yes"));
    assert_eq!(stay_then_local, [ChaosIssueKind::LocalWithStayAtEmitter]);
    let inherited = error_kinds(&one_sequence("local = true\ninherited = XYZ(1, 1, 1)"));
    assert_eq!(inherited, [ChaosIssueKind::InheritedWithLocal]);
    let velocity = error_kinds(&one_sequence("stayAtEmitter = true\ninitial velocity = XYZ(1, 0, 0)"));
    assert_eq!(velocity, [ChaosIssueKind::MotionEventWithStayAtEmitter { event: ChaosEventKind::Velocity }]);
    let location = error_kinds(&one_sequence("stayAtEmitter = true\nchaosLocation = XYZ(1, 0, 0)"));
    assert!(matches!(location.as_slice(), [ChaosIssueKind::SpawnVectorWithStayAtEmitter { .. }]));
    assert_eq!(error_kinds(&one_sequence("collisionCheck = 1")), [ChaosIssueKind::CollisionCheckHangs]);
    let fade_initial = error_kinds(&one_sequence("fade so initial scale = 1"));
    assert_eq!(fade_initial, [ChaosIssueKind::FadeOnInitial]);
}

#[test]
fn false_bool_leaves_the_flag_clear() {
    let desc = desc(&one_sequence("type = point\nlocal = false\nstayAtEmitter = no"));
    assert_eq!(desc.sequences[0].flags, ChaosSequenceFlags::default());
}

#[test]
fn system_block_accepts_only_its_properties() {
    let desc = desc(&script("minEmitTime = random(1, 2)\neventsequence \"s\"\n{\ntype = point\n}\nminEmitTime = 3"));
    assert_eq!(desc.min_emit_time, Range::constant(3.0));
    let kinds = error_kinds(&script("lifetime = 1"));
    assert!(matches!(kinds.as_slice(), [ChaosIssueKind::UnexpectedInSystemBlock { .. }]), "{kinds:?}");
}

#[test]
fn values_follow_the_client_parsers() {
    let desc = desc(&one_sequence(
        "type = Axial\ntexture = \"dir\\\\Flame.DDS\"\nsourceBlendMode = ALPHA_SRCALPHA\n\
         destBlendMode = ALPHA_INVSRCALPHA\nnumParticles = random(10, 20.7)\nburst = 2.9\n\
         celAnimate = CelSetup(16, 0.25, random(0.25, 1), 4, 4, 30, 1)",
    ));
    let sequence = &desc.sequences[0];
    assert_eq!(sequence.particle_type, ChaosParticleType::Axial);
    assert_eq!(sequence.resource, "Flame.DDS");
    assert_eq!(sequence.src_blend, crate::render::BlendFactor::SrcAlpha);
    assert_eq!(sequence.dst_blend, crate::render::BlendFactor::InvSrcAlpha);
    assert_eq!(sequence.num_particles, 20);
    assert_eq!(sequence.burst, IntRange { min: 2, max: 2 });
    assert_eq!(sequence.cel.frames, 16);
    assert_eq!(sequence.cel.v_step, 0.25);
    assert!(sequence.flags.contains(ChaosSequenceFlags::CEL_LOOP));
}

#[test]
fn notes_count_client_quirks() {
    let report = compile(&one_sequence(
        "type = point\nlifetime = 2\nemitRate = random(5, 10)\nburst = random(1, 3)\n\
         initial scale = 1\nfade so at random(0.5, 0.8) scale = 2\ninitial velocity = XYZ(0, random(5, 2), 0)",
    ));
    let has = |predicate: &dyn Fn(&ChaosIssueKind) -> bool| report.issues.iter().any(|issue| predicate(&issue.kind));
    assert!(has(&|kind| matches!(kind, ChaosIssueKind::EmitRateRange { .. })));
    assert!(has(&|kind| matches!(kind, ChaosIssueKind::BurstRange { min: 1, max: 3 })));
    assert!(has(&|kind| matches!(kind, ChaosIssueKind::SharedRandomEventTime { .. })));
    assert!(has(&|kind| matches!(kind, ChaosIssueKind::FadeStepPersists { event: ChaosEventKind::Scale })));
    assert!(has(&|kind| matches!(kind, ChaosIssueKind::ReversedRandom { .. })));
}

// --- Sampling.

#[test]
fn random_ranges_follow_the_client() {
    let mut rng = Mt19937::new(SEED);
    assert_eq!(sample_range(&mut rng, 2.0, 2.0), 2.0);
    for _ in 0..100 {
        let reversed = sample_range(&mut rng, 5.0, 2.0);
        assert!((0.0..=1.0).contains(&reversed));
        let value = sample_range(&mut rng, -1.0, 1.0);
        assert!((-1.0..=1.0).contains(&value));
        let count = sample_int_range(&mut rng, IntRange { min: 1, max: 3 });
        assert!((1..3).contains(&count), "burst max is exclusive: {count}");
        assert_eq!(sample_int_range(&mut rng, IntRange { min: 3, max: 1 }), 0);
    }
}

#[test]
fn event_times_are_drawn_once_per_system() {
    let text = one_sequence("type = point\nlifetime = 10\ninitial scale = 1\nat random(0.2, 0.8) scale = 2\nat 0.5 scale = 3");
    let mut rng = Mt19937::new(SEED);
    let system = ChaosSystem::prepare(desc(&text), &mut rng);
    let times = system.event_times(0);
    assert_eq!(times.len(), 3);
    assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "sorted: {times:?}");
    assert!(times.contains(&0.5));
}

// --- Emission.

#[test]
fn the_first_update_emits_nothing() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 10\nburst = 3"));
    let node = NodeTransform::default();
    run(&mut effect, true, 0.0, 1, STEP_MS, &node);
    assert_eq!(effect.particle_count(), 0);
    effect.update(STEP_MS, &node);
    assert_eq!(effect.particle_count(), 3);
}

#[test]
fn continuous_emission_uses_the_rate_max() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 100\nemitRate = random(1, 4)"));
    // Updates at 1, 2 (burst), 3 (2.25, 2.5, 2.75, 3.0).
    run(&mut effect, true, 0.0, 3, SECOND_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 5);
}

#[test]
fn emission_is_capped_at_sixty_steps_per_second() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 100\nnumParticles = 1000\nemitRate = 1000"));
    run(&mut effect, true, 0.0, 3, SECOND_MS, &NodeTransform::default());
    let continuous = effect.particle_count() - 1;
    assert!((59..=60).contains(&continuous), "{continuous}");
}

#[test]
fn num_particles_caps_the_sequence() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 100\nnumParticles = 7\nemitRate = 50"));
    run(&mut effect, true, 0.0, 5, SECOND_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 7);
}

#[test]
fn long_frames_are_sub_stepped_while_emitting() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 100\nemitRate = 16\ngravity = XYZ(5, 5, -10)"));
    // Updates at 0.25, 0.5 (burst), 0.75: emissions at 0.5625, 0.625, 0.6875 and 0.75 each sub-step the
    // particles born before them.
    run(&mut effect, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let oldest = particles(&effect)[0];
    // Four steps of 1/16 s: z = -10 * (1 + 2 + 3 + 4) / 256. A single step of 0.25 s would give -0.625.
    assert!((oldest.position.z - -0.390625).abs() < EPSILON, "{}", oldest.position.z);
    // Only the vertical gravity component survives.
    assert_eq!(oldest.position.x, 0.0);
    assert_eq!(oldest.velocity[1], 0.0);
}

// --- Events.

#[test]
fn a_fade_without_a_later_fade_keeps_ramping_scale() {
    let mut effect = instance(&one_sequence(
        "type = point\nlifetime = 10\ninitial scale = 1\nfade so at 1 scale = 3\n\
         initial rotation = XYZ(0, 0, 0)\nfade so at 1 rotation = XYZ(0, 90, 0)",
    ));
    // Birth at the second update; then 8 updates of 0.25 s: age 2.
    run(&mut effect, true, 0.0, 10, STEP_MS, &NodeTransform::default());
    let particle = particles(&effect)[0];
    assert!((particle.age - 2.0).abs() < EPSILON);
    // Scale keeps its step of 2 per second after the fade: 1 + 2 * 2.
    assert!((particle.size - 5.0).abs() < EPSILON, "{}", particle.size);
    // Rotation stops at its target.
    assert!((particle.rotation[1] - FRAC_PI_2).abs() < EPSILON, "{}", particle.rotation[1]);
}

#[test]
fn final_fades_over_the_lifetime_max() {
    let mut effect =
        instance(&one_sequence("type = point\nlifetime = random(1, 3)\ninitial scale = 1\nfade so final scale = 0"));
    run(&mut effect, true, 0.0, 2, STEP_MS, &NodeTransform::default());
    let particle = particles(&effect)[0];
    assert!((particle.size_step - -1.0 / 3.0).abs() < EPSILON, "{}", particle.size_step);
}

#[test]
fn reversed_random_draws_from_zero_to_one() {
    let mut effect = instance(&one_sequence(
        "type = point\nlifetime = 10\nburst = 20\nnumParticles = 20\ninitial velocity = XYZ(0, random(5, 2), 0)",
    ));
    run(&mut effect, true, 0.0, 2, STEP_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 20);
    assert!(particles(&effect).iter().all(|particle| (0.0..=1.0).contains(&particle.velocity[1])));
}

#[test]
fn event_timer_rewinds_are_bounded() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 10\nat 0.5 eventTimer = 0.5\nat 0.5 scale = 2"));
    run(&mut effect, true, 0.0, 6, STEP_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 1);
}

// --- Rendering.

#[test]
fn cel_frames_follow_the_particle_age() {
    let mut sequence = ChaosSequenceDesc::new("s");
    sequence.flags.insert(ChaosSequenceFlags::CEL_ANIMATE);
    sequence.cel =
        ChaosCelSetup { frames: 16, u_step: 0.25, v_step: 0.25, columns: 4, rows: 4, fps: 30.0, looping: false };
    let cell = |column: f32, row: f32| UvRect { u0: column * 0.25, v0: row * 0.25, u1: column * 0.25 + 0.25, v1: row * 0.25 + 0.25 };
    assert_eq!(cel_uv(&sequence, 0.1), cell(3.0, 0.0));
    assert_eq!(cel_uv(&sequence, 0.5), cell(3.0, 3.0));
    // Without loop the last frame holds.
    assert_eq!(cel_uv(&sequence, 0.6), cell(3.0, 3.0));
    sequence.flags.insert(ChaosSequenceFlags::CEL_LOOP);
    // 0.6 * 30 = 18 → 18 % 16 = 2.
    assert_eq!(cel_uv(&sequence, 0.6), cell(2.0, 0.0));
}

#[test]
fn local_particles_follow_the_node() {
    let text = script(
        "eventsequence \"world\"\n{\ntype = point\nlifetime = 10\n}\n\
         eventsequence \"local\"\n{\ntype = point\nlifetime = 10\nlocal = true\n}",
    );
    let mut effect = instance(&text);
    let start = NodeTransform::at(Vec3::new(5.0, 0.0, 0.0));
    run(&mut effect, true, 0.0, 2, STEP_MS, &start);
    effect.update(STEP_MS, &NodeTransform::at(Vec3::new(10.0, 0.0, 0.0)));
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    assert_eq!(frame.batches.len(), 2);
    assert_eq!(frame.sprites[0].position.x, 5.0);
    assert_eq!(frame.sprites[1].position.x, 10.0);
}

#[test]
fn axial_sprites_follow_the_motion() {
    let mut effect = instance(&one_sequence("type = axial\nlifetime = 10\ninitial velocity = XYZ(0, 3, 4)\ninitial length = 2"));
    run(&mut effect, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    let batch = &frame.batches[0];
    assert_eq!(batch.kind, SpriteKind::Axial);
    assert!(batch.two_sided);
    let sprite = frame.sprites[0];
    assert!((sprite.direction.y - 0.6).abs() < EPSILON && (sprite.direction.z - 0.8).abs() < EPSILON);
    assert_eq!(sprite.length, 2.0);

    // Without motion an axial particle is not drawn.
    let mut still = instance(&one_sequence("type = axial\nlifetime = 10"));
    run(&mut still, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    still.render(&mut frame);
    assert_eq!(still.particle_count(), 1);
    assert!(frame.sprites.is_empty() && frame.batches.is_empty());
}

#[test]
fn types_without_renderer_draw_nothing() {
    let mut effect = instance(&one_sequence("type = billboard\nlifetime = 10"));
    run(&mut effect, true, 0.0, 2, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    assert_eq!(effect.particle_count(), 1);
    assert!(frame.is_empty());
}

#[test]
fn mesh_particles_become_mesh_instances() {
    let mut effect = instance(&one_sequence("type = mesh\nmeshName = \"bolt.nif\"\nlifetime = 10\ninitial scale = 2"));
    run(&mut effect, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    assert_eq!(frame.meshes.len(), 1);
    assert_eq!(&*frame.meshes[0].mesh, "bolt.nif");
    assert_eq!(frame.meshes[0].scale, 2.0);
    assert!((frame.meshes[0].animation_time - STEP_S).abs() < EPSILON);
}

/// A NIF with one triangle whose material alpha is its controller time over [0, `ALPHA_KEY_TIME`].
fn alpha_keyed_nif() -> Arc<crate::nif::NifEffect> {
    use crate::nif::desc::{
        ControllerTarget, ControllerTiming, Curve, CycleType, FloatInterp, Key, KeyKind, NifController,
        NifControllerKind, NifMaterial, NifMesh, NifNode, NifNodeKind, NifRenderState, NifTransform, Tcb,
    };
    use crate::render::MeshGeometry;
    let key = |time: f32| Key { time, value: time / ALPHA_KEY_TIME, in_tangent: 0.0, out_tangent: 0.0, tcb: Tcb::default() };
    let desc = crate::nif::NifEffectDesc {
        nodes: vec![NifNode {
            name: "bolt".to_owned(),
            parent: None,
            children: Vec::new(),
            transform: NifTransform::IDENTITY,
            flags: 0,
            kind: NifNodeKind::Mesh { mesh: 0 },
            extra_strings: Vec::new(),
            in_scene: true,
            lights: Vec::new(),
            texture_effects: Vec::new(),
            sort_adjust: None,
        }],
        root: Some(0),
        meshes: vec![NifMesh {
            node: 0,
            geometry: MeshGeometry {
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
                ..MeshGeometry::default()
            },
            bound_center: [0.0; 3],
            morph: None,
            render_state: NifRenderState { material: Some(0), ..NifRenderState::default() },
        }],
        materials: vec![NifMaterial::default()],
        controllers: vec![NifController {
            owner: Some(0),
            target: ControllerTarget::Material(0),
            timing: ControllerTiming {
                cycle: CycleType::Clamp,
                active: true,
                app_init: false,
                reverse_direction: false,
                frequency: 1.0,
                phase: 0.0,
                start_time: 0.0,
                stop_time: ALPHA_KEY_TIME,
            },
            kind: NifControllerKind::Alpha(FloatInterp {
                pose: None,
                keys: Some(Curve { kind: KeyKind::Linear, keys: vec![key(0.0), key(ALPHA_KEY_TIME)] }),
            }),
        }],
        ..crate::nif::NifEffectDesc::default()
    };
    Arc::new(crate::nif::NifEffect::prepare(desc))
}

/// End of the alpha keys of [`alpha_keyed_nif`].
const ALPHA_KEY_TIME: f32 = 4.0;

#[test]
fn mesh_particles_draw_a_clone_of_their_nif_at_their_age() {
    const SIZE: f32 = 2.0;
    let mut effect = instance(&one_sequence(&format!(
        "type = mesh\nmeshName = \"bolt.nif\"\nlifetime = 10\nburst = 2\ninitial scale = {SIZE}\n\
         initial rotation = XYZ(0, 0, 90)\ninitial velocity = XYZ(1, 0, 0)"
    )));
    let nif = alpha_keyed_nif();
    let resolved = Arc::clone(&nif);
    effect.set_mesh_source(&MeshSource::new(move |name| (name == "bolt.nif").then(|| Arc::clone(&resolved))));
    run(&mut effect, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    assert!(frame.meshes.is_empty(), "resolved meshes draw no placements");
    let live = particles(&effect);
    assert_eq!(frame.mesh_draws.len(), live.len());
    assert!(live.len() >= 2);
    for (draw, particle) in frame.mesh_draws.iter().zip(live) {
        // The NIF's own material, its controllers at the particle age; no particle colour.
        assert!((draw.material.alpha - particle.age / ALPHA_KEY_TIME).abs() < EPSILON, "{draw:?} {particle:?}");
        assert!((draw.translation - particle.position).length() < EPSILON);
        assert_eq!(draw.scale, SIZE);
        // Rotation flag set: Euler(rotation) · node rotation (identity).
        let expected = euler_xyz(particle.rotation);
        assert_eq!(draw.rotation, expected);
        // Each clone geometry is its own sort item, at its world bound centre.
        assert_eq!(draw.sort.center, particle.position);
    }
    // Clones keep the deque order and their own scene order numbers.
    assert!(frame.mesh_draws.windows(2).all(|pair| pair[0].order < pair[1].order));
    // A name the source does not resolve keeps drawing placements.
    let mut unresolved = instance(&one_sequence("type = mesh\nmeshName = \"other.nif\"\nlifetime = 10"));
    unresolved.set_mesh_source(&MeshSource::new(move |name| (name == "bolt.nif").then(|| Arc::clone(&nif))));
    run(&mut unresolved, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    unresolved.render(&mut frame);
    assert_eq!(frame.meshes.len(), 1);
    assert!(frame.mesh_draws.is_empty());
}

#[test]
fn euler_rotations_follow_the_sdk_xyz_order() {
    use crate::math::Mat3;
    // A quarter turn about z alone maps x to -y in the SDK's (row-major, M · v) matrices.
    let z = euler_xyz([0.0, 0.0, FRAC_PI_2]);
    assert!((z.mul_vec3(Vec3::X) - Vec3::new(0.0, -1.0, 0.0)).length() < EPSILON);
    // X · (Y · Z): z applies first.
    let xz = euler_xyz([FRAC_PI_2, 0.0, FRAC_PI_2]);
    let expected = euler_xyz([FRAC_PI_2, 0.0, 0.0]).mul_mat3(&z);
    let close = |a: &Mat3, b: &Mat3| a.rows.iter().flatten().zip(b.rows.iter().flatten()).all(|(x, y)| (x - y).abs() < EPSILON);
    assert!(close(&xz, &expected));
}

#[test]
fn quad_sequences_sort_by_the_box_centre_of_their_particles() {
    let mut effect = instance(&one_sequence(
        "type = point\nlifetime = 10\nburst = 3\ninitial velocity = XYZ(random(-4, 4), random(-4, 4), random(-4, 4))",
    ));
    run(&mut effect, true, 0.0, 3, STEP_MS, &NodeTransform::default());
    let mut frame = RenderFrame::default();
    effect.render(&mut frame);
    let live = particles(&effect);
    let axis = |f: fn(&runtime::ChaosParticle) -> f32| {
        let values = live.iter().map(f);
        (values.clone().fold(f32::INFINITY, f32::min) + values.fold(f32::NEG_INFINITY, f32::max)) * 0.5
    };
    let center = Vec3::new(axis(|p| p.position.x), axis(|p| p.position.y), axis(|p| p.position.z));
    let batch = &frame.batches[0];
    assert!(batch.sort.sorted);
    assert!((batch.sort.center - center).length() < EPSILON, "{:?} {center:?}", batch.sort);
    let (bound_center, radius) = effect.emitter().bound(0).unwrap();
    assert_eq!(bound_center, batch.sort.center);
    let farthest = live.iter().map(|p| (p.position - center).length()).fold(0.0, f32::max);
    assert!((radius - (farthest + BOUND_RADIUS_MARGIN)).abs() < EPSILON);
}

// --- Start and stop.

#[test]
fn a_one_shot_effect_bursts_once_and_finishes() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 1\nemitRate = 100\nburst = 2"));
    run(&mut effect, false, 0.0, 2, STEP_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 2);
    assert!(!effect.emitter().is_active());
    assert!(effect.is_finished());
    assert!(!effect.is_drained());
    for _ in 0..5 {
        effect.update(STEP_MS, &NodeTransform::default());
    }
    assert!(effect.is_drained());
}

#[test]
fn min_emit_time_sets_the_emission_time_of_non_looping_effects() {
    let mut effect = instance(&script("minEmitTime = 1\neventsequence \"s\"\n{\ntype = point\nlifetime = 100\nemitRate = 4\n}"));
    run(&mut effect, false, 0.0, 2, STEP_MS, &NodeTransform::default());
    assert!(effect.emitter().is_active());
    // 1 s of emission, counted from the second update: remaining reaches 0 at the fifth.
    for _ in 0..2 {
        effect.update(STEP_MS, &NodeTransform::default());
    }
    assert!(effect.emitter().is_active());
    effect.update(STEP_MS, &NodeTransform::default());
    assert!(!effect.emitter().is_active());
    let count = effect.particle_count();
    effect.update(STEP_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), count);
}

#[test]
fn stop_kills_only_stay_at_emitter_particles() {
    let text = script(
        "eventsequence \"free\"\n{\ntype = point\nlifetime = 10\n}\n\
         eventsequence \"stay\"\n{\ntype = point\nlifetime = 10\nstayAtEmitter = true\n}",
    );
    let mut effect = instance(&text);
    run(&mut effect, true, 0.0, 2, STEP_MS, &NodeTransform::default());
    assert_eq!(effect.particle_count(), 2);
    effect.stop();
    assert_eq!(effect.particle_count(), 1);
    assert!(!effect.is_finished());
    for _ in 0..(EFFECT_FADE_SECONDS / STEP_S) as usize {
        effect.update(STEP_MS, &NodeTransform::default());
    }
    assert!(effect.is_finished());
}

#[test]
fn lifetime_ticks_finish_the_effect() {
    let mut effect = instance(&one_sequence("type = point\nlifetime = 10"));
    effect.set_lifetime_ticks(Some(2));
    run(&mut effect, true, 0.0, 1, STEP_MS, &NodeTransform::default());
    assert!(!effect.is_finished());
    effect.update(STEP_MS, &NodeTransform::default());
    assert!(effect.is_finished());
}

#[test]
fn a_seed_replays_the_same_effect() {
    let text = one_sequence(
        "type = point\nlifetime = random(1, 2)\nemitRate = 30\nchaosLocation = XYZ(random(-1, 1), 0, random(0, 2))\n\
         initial velocity = XYZ(random(-1, 1), random(-1, 1), 1)",
    );
    let positions = |text: &str| {
        let mut effect = instance(text);
        run(&mut effect, true, 0.0, 20, STEP_MS, &NodeTransform::default());
        particles(&effect).iter().map(|particle| particle.position).collect::<Vec<_>>()
    };
    let first = positions(&text);
    assert!(!first.is_empty());
    assert_eq!(first, positions(&text));
}

#[test]
fn spawn_uses_node_rotation_and_scale() {
    let mut effect = instance(&one_sequence(
        "type = point\nlifetime = 10\nchaosLocation = XYZ(1, 0, 0)\ninitial velocity = XYZ(1, 0, 0)\ninitial scale = 1",
    ));
    // A quarter turn about z: x goes to y.
    let rotation = crate::math::Mat3::from_rows([[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    let node = NodeTransform { position: Vec3::new(10.0, 0.0, 0.0), rotation, scale: 2.0, velocity: Some(Vec3::ZERO) };
    run(&mut effect, true, 0.0, 2, STEP_MS, &node);
    let particle = particles(&effect)[0];
    assert_eq!(particle.position, Vec3::new(10.0, 2.0, 0.0));
    assert_eq!(particle.velocity, [0.0, 2.0, 0.0]);
    assert_eq!(particle.size, 2.0);
}
