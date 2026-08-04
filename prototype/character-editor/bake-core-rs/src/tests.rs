use std::alloc::System;

use crate::*;

mod allocation;
mod short04_golden;

use allocation::{TrackingAllocator, assert_no_alloc};

#[global_allocator]
static ALLOCATOR: TrackingAllocator<System> = TrackingAllocator::new(System);

fn close(left: f32, right: f32) {
    assert!((left - right).abs() <= 1.0e-6, "{left} != {right}");
}

fn close3(left: [f32; 3], right: [f32; 3]) {
    close(left[0], right[0]);
    close(left[1], right[1]);
    close(left[2], right[2]);
}

#[test]
fn surface_wrap_preserves_signed_weights_and_scales_offsets() {
    let driver = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [0.0, 6.0, 0.0],
        [0.0, 0.0, 8.0],
    ];
    let scale = SurfaceScale {
        x: Some(AxisScale {
            minimum_vertex: 0,
            maximum_vertex: 1,
            source_distance: 2.0,
        }),
        y: Some(AxisScale {
            minimum_vertex: 0,
            maximum_vertex: 2,
            source_distance: 3.0,
        }),
        z: Some(AxisScale {
            minimum_vertex: 0,
            maximum_vertex: 3,
            source_distance: 4.0,
        }),
    };
    let bindings = [
        SurfaceBinding {
            driver_vertices: [0, 1, 2],
            weights: [-0.25, 1.0, 0.25],
            offset: [0.5, -0.5, 1.0],
        },
        SurfaceBinding::exact(3),
    ];
    let mut output = [[0.0; 3]; 2];
    fit_surface(&driver, &bindings, scale, &mut output).unwrap();
    close3(output[0], [5.0, 0.5, 2.0]);
    close3(output[1], driver[3]);
}

#[test]
fn surface_wrap_matches_mpfb_short04_neutral_and_head_width_golden_data() {
    use short04_golden::{
        BINDINGS, MORPH_DRIVER, MORPH_EXPECTED, NEUTRAL_DRIVER, NEUTRAL_EXPECTED, SCALE,
        SOURCE_VERTICES,
    };

    assert_eq!(SOURCE_VERTICES.len(), NEUTRAL_DRIVER.len());
    assert!(
        BINDINGS
            .iter()
            .any(|binding| binding.weights.iter().any(|weight| *weight < 0.0))
    );
    assert!(
        BINDINGS
            .iter()
            .any(|binding| binding.weights.iter().any(|weight| *weight > 1.0))
    );
    let mut output = [[0.0; 3]; BINDINGS.len()];
    fit_surface(&NEUTRAL_DRIVER, &BINDINGS, SCALE, &mut output).unwrap();
    for (actual, expected) in output.iter().zip(NEUTRAL_EXPECTED) {
        for axis in 0..3 {
            assert!((actual[axis] - expected[axis]).abs() <= 3.0e-6);
        }
    }

    fit_surface(&MORPH_DRIVER, &BINDINGS, SCALE, &mut output).unwrap();
    for (actual, expected) in output.iter().zip(MORPH_EXPECTED) {
        for axis in 0..3 {
            assert!((actual[axis] - expected[axis]).abs() <= 3.0e-6);
        }
    }
}

#[test]
fn surface_wrap_rejects_bad_input_before_output_changes() {
    let driver = [[0.0, 0.0, 0.0]];
    let bindings = [SurfaceBinding::exact(2)];
    let mut output = [[9.0, 8.0, 7.0]];
    assert_eq!(
        fit_surface(&driver, &bindings, SurfaceScale::default(), &mut output),
        Err(CharacterBakeError::IndexOutOfRange),
    );
    assert_eq!(output, [[9.0, 8.0, 7.0]]);
}

#[test]
fn surface_wrap_overflow_does_not_publish_a_prefix() {
    let driver = [[f32::MAX, 0.0, 0.0]];
    let bindings = [
        SurfaceBinding::exact(0),
        SurfaceBinding {
            driver_vertices: [0; 3],
            weights: [2.0, 0.0, 0.0],
            offset: [0.0; 3],
        },
    ];
    let mut output = [[7.0; 3]; 2];
    assert_eq!(
        fit_surface(&driver, &bindings, SurfaceScale::default(), &mut output),
        Err(CharacterBakeError::NonFiniteValue),
    );
    assert_eq!(output, [[7.0; 3]; 2]);
}

#[test]
fn sparse_targets_build_and_update_the_same_shape() {
    let neutral = [[0.0, 0.0, 0.0], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let first_deltas = [
        SparseDelta {
            vertex: 0,
            delta: [2.0, 0.0, 0.0],
        },
        SparseDelta {
            vertex: 2,
            delta: [0.0, 4.0, 0.0],
        },
    ];
    let second_deltas = [SparseDelta {
        vertex: 1,
        delta: [0.0, 0.0, -2.0],
    }];
    let first = SparseTarget {
        deltas: &first_deltas,
    };
    let second = SparseTarget {
        deltas: &second_deltas,
    };
    let mut output = [[0.0; 3]; 3];
    evaluate_sparse_targets(&neutral, &[first, second], &[1.5, -0.5], &mut output).unwrap();
    assert_eq!(output, [[3.0, 0.0, 0.0], [1.0, 2.0, 4.0], [4.0, 11.0, 6.0]]);

    apply_sparse_target_delta(&mut output, first, 1.5, 0.25).unwrap();
    let mut expected = [[0.0; 3]; 3];
    evaluate_sparse_targets(&neutral, &[first, second], &[0.25, -0.5], &mut expected).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn sparse_targets_require_unique_ascending_vertices() {
    let deltas = [
        SparseDelta {
            vertex: 1,
            delta: [1.0, 0.0, 0.0],
        },
        SparseDelta {
            vertex: 1,
            delta: [0.0, 1.0, 0.0],
        },
    ];
    let mut output = [[7.0; 3]; 2];
    assert_eq!(
        evaluate_sparse_targets(
            &[[0.0; 3]; 2],
            &[SparseTarget { deltas: &deltas }],
            &[1.0],
            &mut output,
        ),
        Err(CharacterBakeError::InvalidSparseTarget),
    );
    assert_eq!(output, [[7.0; 3]; 2]);
}

#[test]
fn piecewise_macros_resolve_boundaries_and_products() {
    let segments = [
        MacroSegment {
            lowest: 0.0,
            highest: 0.5,
            low_state: 0,
            high_state: 1,
        },
        MacroSegment {
            lowest: 0.5,
            highest: 1.0,
            low_state: 1,
            high_state: 2,
        },
    ];
    let mut states = [0.0; 5];
    resolve_piecewise_macro(0.75, &segments, &mut states).unwrap();
    assert_eq!(states, [0.0, 0.5, 0.5, 0.0, 0.0]);
    states[3] = 0.25;
    states[4] = 0.75;

    let factors = [1, 3, 2, 4];
    let terms = [
        MacroProductTerm {
            target: 0,
            first_factor: 0,
            factor_count: 2,
        },
        MacroProductTerm {
            target: 1,
            first_factor: 2,
            factor_count: 2,
        },
    ];
    let mut targets = [0.0; 2];
    compose_macro_products(&states, &factors, &terms, &mut targets).unwrap();
    close(targets[0], 0.125);
    close(targets[1], 0.375);

    resolve_piecewise_macro(0.5, &segments, &mut states).unwrap();
    assert_eq!(&states[..3], &[0.0, 1.0, 0.0]);
}

#[test]
fn piecewise_macro_supports_an_empty_endpoint() {
    let segments = [MacroSegment {
        lowest: 0.0,
        highest: 1.0,
        low_state: NO_MACRO_STATE,
        high_state: 0,
    }];
    let mut states = [0.0; 1];
    resolve_piecewise_macro(0.25, &segments, &mut states).unwrap();
    close(states[0], 0.25);
}

#[test]
fn skin_transfer_aggregates_and_does_not_repeat_padding() {
    let driver = [
        SkinInfluences {
            joints: [1, 2, 0, 0],
            weights: [0.6, 0.4, 0.0, 0.0],
        },
        SkinInfluences {
            joints: [2, 3, 0, 0],
            weights: [0.5, 0.5, 0.0, 0.0],
        },
    ];
    let bindings = [SurfaceBinding {
        driver_vertices: [0, 1, 0],
        weights: [0.5, 0.5, 0.0],
        offset: [0.0; 3],
    }];
    let mut output = [SkinInfluences::default()];
    transfer_skin_weights(&driver, &bindings, &mut output).unwrap();
    assert_eq!(output[0].joints, [2, 1, 3, 0]);
    close(output[0].weights[0], 0.45);
    close(output[0].weights[1], 0.30);
    close(output[0].weights[2], 0.25);
    close(output[0].weights[3], 0.0);
}

#[test]
fn skin_transfer_clamps_negative_aggregates_after_signed_mapping() {
    let driver = [
        SkinInfluences {
            joints: [1, 2, 0, 0],
            weights: [0.8, 0.2, 0.0, 0.0],
        },
        SkinInfluences {
            joints: [1, 2, 3, 0],
            weights: [0.2, 0.1, 0.7, 0.0],
        },
    ];
    let bindings = [SurfaceBinding {
        driver_vertices: [0, 1, 0],
        weights: [-0.5, 1.5, 0.0],
        offset: [0.0; 3],
    }];
    let mut output = [SkinInfluences::default()];
    transfer_skin_weights(&driver, &bindings, &mut output).unwrap();
    assert_eq!(output[0].joints[..2], [3, 2]);
    close(output[0].weights[0], 1.05 / 1.10);
    close(output[0].weights[1], 0.05 / 1.10);
    assert_eq!(output[0].weights[2..], [0.0, 0.0]);
}

#[test]
fn skin_transfer_selects_top_four_with_stable_ties() {
    let driver = [
        SkinInfluences {
            joints: [9, 7, 5, 3],
            weights: [0.25; 4],
        },
        SkinInfluences {
            joints: [8, 6, 4, 2],
            weights: [0.25; 4],
        },
    ];
    let bindings = [SurfaceBinding {
        driver_vertices: [0, 1, 0],
        weights: [0.5, 0.5, 0.0],
        offset: [0.0; 3],
    }];
    let mut output = [SkinInfluences::default()];
    transfer_skin_weights(&driver, &bindings, &mut output).unwrap();
    assert_eq!(output[0].joints, [2, 3, 4, 5]);
    assert_eq!(output[0].weights, [0.25; 4]);
}

#[test]
fn skin_transfer_failure_does_not_publish_a_prefix() {
    let driver = [SkinInfluences::default()];
    let bindings = [SurfaceBinding::exact(0), SurfaceBinding::exact(0)];
    let original = SkinInfluences {
        joints: [4, 3, 2, 1],
        weights: [0.4, 0.3, 0.2, 0.1],
    };
    let mut output = [original; 2];
    assert_eq!(
        transfer_skin_weights(&driver, &bindings, &mut output),
        Err(CharacterBakeError::MissingSkinInfluence),
    );
    assert_eq!(output, [original; 2]);
}

#[test]
fn normals_are_area_weighted_and_report_degenerate_faces() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [2.0, 2.0, 2.0],
    ];
    let indices = [0, 1, 2, 0, 2, 3, 0, 0, 0];
    let mut normals = [[9.0; 3]; 5];
    let stats = rebuild_area_weighted_normals(&positions, &indices, &mut normals).unwrap();
    assert_eq!(
        stats,
        NormalBuildStats {
            triangles: 3,
            degenerate_triangles: 1,
            isolated_vertices: 1
        }
    );
    for normal in &normals[..4] {
        close3(*normal, [0.0, 0.0, 1.0]);
    }
    assert_eq!(normals[4], [0.0; 3]);
}

#[test]
fn hot_algorithms_do_not_allocate() {
    let driver_positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let binding = SurfaceBinding {
        driver_vertices: [0, 1, 2],
        weights: [0.2, 0.3, 0.5],
        offset: [0.0; 3],
    };
    let mut fitted = [[0.0; 3]];
    let delta = [SparseDelta {
        vertex: 1,
        delta: [0.5, 0.0, 0.0],
    }];
    let target = SparseTarget { deltas: &delta };
    let mut morphed = driver_positions;
    let driver_skin = [SkinInfluences {
        joints: [0, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
    }; 3];
    let mut fitted_skin = [SkinInfluences::default()];
    let mut macro_states = [0.0; 2];
    let segments = [MacroSegment {
        lowest: 0.0,
        highest: 1.0,
        low_state: 0,
        high_state: 1,
    }];
    let mut normals = [[0.0; 3]; 3];

    assert_no_alloc(|| {
        evaluate_sparse_targets(&driver_positions, &[target], &[0.5], &mut morphed).unwrap();
        apply_sparse_target_delta(&mut morphed, target, 0.5, 0.75).unwrap();
        fit_surface(&morphed, &[binding], SurfaceScale::default(), &mut fitted).unwrap();
        transfer_skin_weights(&driver_skin, &[binding], &mut fitted_skin).unwrap();
        resolve_piecewise_macro(0.25, &segments, &mut macro_states).unwrap();
        rebuild_area_weighted_normals(&driver_positions, &[0, 1, 2], &mut normals).unwrap();
    });
}
