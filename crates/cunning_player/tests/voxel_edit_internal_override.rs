use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use cunning_cda_runtime::asset::{ConnectionDef, NodeDef, PortDef, RuntimeDefinition, RuntimeMeta};
use cunning_cda_runtime::registry::RuntimeRegistry;
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
use cunning_kernel::mesh::Geometry;
use cunning_kernel::traits::parameter::ParameterValue;
use uuid::Uuid;

fn make_def(cmds_json: &str) -> RuntimeDefinition {
    // Deterministic IDs so we can reuse a compiled ExecutionPlan across defs
    // while varying only parameter values.
    let n_in = Uuid::from_u128(1);
    let n_vox = Uuid::from_u128(2);
    let n_out = Uuid::from_u128(3);

    let mut in_params = HashMap::new();
    in_params.insert("index".to_string(), ParameterValue::Int(0));

    let mut vox_params = HashMap::new();
    vox_params.insert("voxel_size".to_string(), ParameterValue::Float(0.2));
    vox_params.insert(
        "cmds_json".to_string(),
        ParameterValue::String(cmds_json.to_string()),
    );
    vox_params.insert(
        "palette_json".to_string(),
        ParameterValue::String("[]".to_string()),
    );

    RuntimeDefinition {
        meta: RuntimeMeta {
            format_version: 1,
            min_engine_version: "0.1.0".to_string(),
            uuid: Uuid::from_u128(0xCDA0_0000_0000_0000_0000_0000_0000_0001),
            name: "test-voxel-edit".to_string(),
            author: None,
            license: None,
        },
        inputs: vec![PortDef {
            id: Uuid::from_u128(0x1000),
            name: "in0".to_string(),
            data_type: "Geometry".to_string(),
        }],
        outputs: vec![PortDef {
            id: Uuid::from_u128(0x2000),
            name: "out0".to_string(),
            data_type: "Geometry".to_string(),
        }],
        nodes: vec![
            NodeDef {
                id: n_in,
                type_id: "cunning.input".to_string(),
                params: in_params,
            },
            NodeDef {
                id: n_vox,
                type_id: "cunning.voxel.edit".to_string(),
                params: vox_params,
            },
            NodeDef {
                id: n_out,
                type_id: "cunning.output".to_string(),
                params: {
                    let mut p = HashMap::new();
                    // Required by runtime compiler: which exported output this node binds to.
                    p.insert(
                        "name".to_string(),
                        ParameterValue::String("out0".to_string()),
                    );
                    p
                },
            },
        ],
        connections: vec![
            // input(out:0=0) -> voxel(in:0=0)
            ConnectionDef {
                id: Uuid::from_u128(0x3000),
                from_node: n_in,
                from_port: 0,
                to_node: n_vox,
                to_port: 0,
                order: 0,
            },
            // voxel(out:0=0) -> output(in:0=0)
            ConnectionDef {
                id: Uuid::from_u128(0x3001),
                from_node: n_vox,
                from_port: 0,
                to_node: n_out,
                to_port: 0,
                order: 0,
            },
        ],
        promoted_params: vec![],
        hud_units: vec![],
        coverlay_units: vec![],
    }
}

#[test]
fn voxel_edit_cmds_json_affects_output() {
    let reg = RuntimeRegistry::new_default();

    // External input: empty geometry (so voxel edit base isn't auto-filled from input).
    let external_inputs: Vec<Arc<Geometry>> = vec![Arc::new(Geometry::new())];

    let def0 = make_def(&vox::DiscreteVoxelCmdList::default().to_json());
    let plan = cunning_cda_runtime::compiler::compile(&def0, &reg).expect("compile");

    // Case A: empty cmdlist -> empty output.
    let outs0 = cunning_cda_runtime::vm::execute(
        &plan,
        &def0,
        &reg,
        &external_inputs,
        &HashMap::new(),
        &AtomicBool::new(false),
    )
    .expect("cook");
    let prims_a = outs0
        .first()
        .map(|g| g.primitives().len())
        .unwrap_or(0);
    let cells_a = outs0
        .first()
        .and_then(|g| g.get_detail_attribute("__voxel_cells_i32"))
        .and_then(|a| a.as_slice::<i32>())
        .map(|s| s.len())
        .unwrap_or(0);

    // Case B: add a single voxel at origin.
    let mut cmds = vox::DiscreteVoxelCmdList::default();
    cmds.push(vox::DiscreteVoxelOp::SetVoxel {
        x: 0,
        y: 0,
        z: 0,
        palette_index: 1,
    });
    // Sanity: JSON roundtrip and bake must create at least one solid voxel.
    let cmds_rt = vox::DiscreteVoxelCmdList::from_json(&cmds.to_json());
    assert_eq!(cmds_rt.cursor, 1);
    assert_eq!(cmds_rt.ops.len(), 1);
    let mut grid = vox::DiscreteVoxelGrid::new(0.2);
    let mut st = vox::discrete::DiscreteBakeState::default();
    vox::discrete::bake_cmds_incremental(&mut grid, &cmds_rt, &mut st);
    assert!(
        !grid.voxels.is_empty(),
        "expected baked grid to contain voxels"
    );

    let def1 = make_def(&cmds.to_json());
    let outs1 = cunning_cda_runtime::vm::execute(
        &plan,
        &def1,
        &reg,
        &external_inputs,
        &HashMap::new(),
        &AtomicBool::new(false),
    )
    .expect("cook");
    let prims_b = outs1
        .first()
        .map(|g| g.primitives().len())
        .unwrap_or(0);
    let cells_b = outs1
        .first()
        .and_then(|g| g.get_detail_attribute("__voxel_cells_i32"))
        .and_then(|a| a.as_slice::<i32>())
        .map(|s| s.len())
        .unwrap_or(0);

    // If runtime path doesn't change, directly test the node compute path.
    if prims_b == prims_a && cells_b == cells_a {
        let mut pm: HashMap<String, ParameterValue> = HashMap::new();
        pm.insert("voxel_size".to_string(), ParameterValue::Float(0.2));
        pm.insert(
            "cmds_json".to_string(),
            ParameterValue::String(cmds.to_json()),
        );
        pm.insert(
            "palette_json".to_string(),
            ParameterValue::String("[]".to_string()),
        );
        let out = cunning_kernel::nodes::voxel::voxel_edit::compute_voxel_edit(
            None,
            &Geometry::new(),
            &pm,
        );
        let direct_cells = out
            .get_detail_attribute("__voxel_cells_i32")
            .and_then(|a| a.as_slice::<i32>())
            .map(|s| s.len())
            .unwrap_or(0);
        assert!(
            direct_cells > 0,
            "direct compute_voxel_edit produced no voxel payload; expected __voxel_cells_i32"
        );
    }

    assert!(
        prims_b > prims_a || cells_b > cells_a,
        "expected voxel add to change output (prims {prims_a}->{prims_b}, cells {cells_a}->{cells_b})"
    );
}

