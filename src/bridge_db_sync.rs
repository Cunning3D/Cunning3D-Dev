use bevy::camera::{Projection, ScalingMode};
use bevy::ecs::prelude::MessageWriter;
use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task};
use futures_lite::future;
use std::sync::Arc;

use cunning_kernel::coord::basis::{self, BasisId};
use cunning_kernel::io::blob_store::BlobStore;
use cunning_kernel::spline_snapshot_fbs;

#[derive(Resource, Default, Clone)]
pub struct BridgeDb {
    pub path: Option<String>,
}

#[derive(Resource)]
struct BridgeDbStore {
    store: Arc<BlobStore>,
}

#[derive(Resource, Default)]
struct BridgeDbState {
    last_unity_blob: u64,
    last_cunning_hash: u64,
    last_spline_blob: std::collections::HashMap<uuid::Uuid, u64>,
}

const K_UNITY: &[u8] = b"viewport.unity";
const K_CUNNING: &[u8] = b"viewport.cunning";

// Bridge sync can touch disk-backed DB (redb) and may hitch on Windows.
// Throttle to avoid per-frame I/O and lock contention.
const BRIDGE_PULL_VIEWPORT_HZ: f32 = 30.0;
const BRIDGE_PUSH_VIEWPORT_HZ: f32 = 15.0;
const BRIDGE_PULL_SPLINES_HZ: f32 = 5.0;

#[derive(Default)]
struct BridgeTick {
    timer: Option<Timer>,
}

fn bridge_tick(timer: &mut Option<Timer>, time: &Time) -> bool {
    let Some(t) = timer.as_mut() else {
        return false;
    };
    t.tick(time.delta());
    t.just_finished()
}

#[inline]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn open_bridge_db_on_enter(mut commands: Commands, bridge: Res<BridgeDb>) {
    let Some(p) = bridge.path.as_deref() else {
        return;
    };
    if p.is_empty() {
        return;
    }
    let store = match BlobStore::open_or_create(p.into()) {
        Ok(s) => s,
        Err(e) => {
            warn!("bridge-db open failed: {e}");
            return;
        }
    };
    commands.insert_resource(BridgeDbStore {
        store: Arc::new(store),
    });
    commands.init_resource::<BridgeDbState>();
}

fn pull_unity_viewport(
    store: Option<Res<BridgeDbStore>>,
    st: Option<ResMut<BridgeDbState>>,
    mut q: Query<(&mut Transform, &mut Projection), With<crate::MainCamera>>,
    nav_input: Res<crate::input::NavigationInput>,
    viewport_interaction: Res<crate::ViewportInteractionState>,
    time: Res<Time>,
    mut tick: Local<BridgeTick>,
    mut task: Local<Option<Task<Option<(u64, Vec<u8>)>>>>,
    mut pending: Local<Option<(u64, Vec<u8>)>>,
) {
    let Some(store) = store else {
        return;
    };
    let Some(mut st) = st else {
        return;
    };
    let Ok((mut tr, mut proj)) = q.single_mut() else {
        return;
    };

    // Poll async DB read results first (non-blocking).
    if let Some(t) = &mut *task {
        if let Some(res) = future::block_on(future::poll_once(t)) {
            *task = None;
            if let Some((blob_id, bytes)) = res {
                *pending = Some((blob_id, bytes));
            }
        }
    }

    if tick.timer.is_none() {
        tick.timer = Some(Timer::from_seconds(
            1.0 / BRIDGE_PULL_VIEWPORT_HZ,
            TimerMode::Repeating,
        ));
    }
    if task.is_none() && bridge_tick(&mut tick.timer, &time) {
        let store = store.store.clone();
        *task = Some(IoTaskPool::get().spawn(async move {
            let key = fnv1a64(K_UNITY);
            let blob_id = store.get_latest(key).ok().unwrap_or(0);
            if blob_id == 0 {
                return None;
            }
            let bytes = store.get(blob_id).ok().flatten()?;
            Some((blob_id, bytes))
        }));
    }

    // Never let an external bridge overwrite the local camera while the user is interacting.
    if nav_input.active
        || nav_input.zoom_delta != 0.0
        || nav_input.orbit_delta.length_squared() != 0.0
        || nav_input.pan_delta.length_squared() != 0.0
        || nav_input.fly_vector.length_squared() != 0.0
        || viewport_interaction.is_gizmo_dragging
        || viewport_interaction.is_right_button_dragged
        || viewport_interaction.is_middle_button_dragged
        || viewport_interaction.is_alt_left_button_dragged
    {
        return;
    }

    let Some((blob_id, bytes)) = pending.take() else {
        return;
    };

    if blob_id == 0 || blob_id == st.last_unity_blob {
        return;
    }

    if bytes.len() < 64 {
        return;
    }
    st.last_unity_blob = blob_id;

    let ver = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([0; 4]));
    if ver != 1 {
        return;
    }
    let basis_id = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
    let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4]));
    let fov = f32::from_le_bytes(bytes[16..20].try_into().unwrap_or([0; 4]));
    let ortho_size = f32::from_le_bytes(bytes[20..24].try_into().unwrap_or([0; 4]));
    let near = f32::from_le_bytes(bytes[24..28].try_into().unwrap_or([0; 4]));
    let far = f32::from_le_bytes(bytes[28..32].try_into().unwrap_or([0; 4]));
    let pos = Vec3::new(
        f32::from_le_bytes(bytes[32..36].try_into().unwrap_or([0; 4])),
        f32::from_le_bytes(bytes[36..40].try_into().unwrap_or([0; 4])),
        f32::from_le_bytes(bytes[40..44].try_into().unwrap_or([0; 4])),
    );
    let rot = Quat::from_xyzw(
        f32::from_le_bytes(bytes[44..48].try_into().unwrap_or([0; 4])),
        f32::from_le_bytes(bytes[48..52].try_into().unwrap_or([0; 4])),
        f32::from_le_bytes(bytes[52..56].try_into().unwrap_or([0; 4])),
        f32::from_le_bytes(bytes[56..60].try_into().unwrap_or([0; 4])),
    );

    // Map Unity basis to internal Bevy basis at boundary.
    let (from, to) = if basis_id == 1 {
        (BasisId::Unity, BasisId::InternalBevy)
    } else {
        (BasisId::InternalBevy, BasisId::InternalBevy)
    };
    let map = basis::map(from, to);
    tr.translation = map.map(|m| m.map_v3(pos)).unwrap_or(pos);
    tr.rotation = map.map(|m| m.map_q(rot)).unwrap_or(rot);

    let is_ortho = (flags & 1) != 0;
    if is_ortho {
        let mut o = OrthographicProjection::default_3d();
        o.near = near;
        o.far = far;
        o.scale = 1.0;
        o.scaling_mode = ScalingMode::FixedVertical {
            viewport_height: ortho_size * 2.0,
        };
        *proj = Projection::Orthographic(o);
    } else {
        let mut p = PerspectiveProjection::default();
        p.fov = fov.to_radians();
        p.near = near.max(1e-4);
        p.far = far.max(p.near + 1e-3);
        *proj = Projection::Perspective(p);
    }
}

fn push_cunning_viewport(
    store: Option<Res<BridgeDbStore>>,
    st: Option<ResMut<BridgeDbState>>,
    q: Query<(&Transform, &Projection), With<crate::MainCamera>>,
    time: Res<Time>,
    mut tick: Local<BridgeTick>,
    mut task: Local<Option<Task<bool>>>,
    mut inflight_hash: Local<Option<u64>>,
) {
    let Some(store) = store else {
        return;
    };
    let Some(mut st) = st else {
        return;
    };
    let Ok((tr, proj)) = q.single() else {
        return;
    };

    // Poll async DB write completion first.
    if let Some(t) = &mut *task {
        if let Some(ok) = future::block_on(future::poll_once(t)) {
            *task = None;
            if ok {
                if let Some(h) = inflight_hash.take() {
                    st.last_cunning_hash = h;
                }
            } else {
                inflight_hash.take();
            }
        }
    }

    if tick.timer.is_none() {
        tick.timer = Some(Timer::from_seconds(
            1.0 / BRIDGE_PUSH_VIEWPORT_HZ,
            TimerMode::Repeating,
        ));
    }
    if !bridge_tick(&mut tick.timer, &time) || task.is_some() {
        return;
    }

    let (flags, fov_deg, ortho_size, near, far): (u32, f32, f32, f32, f32) = match proj {
        Projection::Perspective(p) => (0u32, p.fov.to_degrees(), 0.0f32, p.near, p.far),
        Projection::Orthographic(o) => (1u32, 60.0f32, 1.0f32, o.near, o.far),
        _ => (0u32, 60.0f32, 0.0f32, 0.01f32, 10000.0f32),
    };

    let mut b = [0u8; 64];
    b[0..4].copy_from_slice(&1u32.to_le_bytes());
    b[4..8].copy_from_slice(&0u32.to_le_bytes()); // basis=internal
    b[8..12].copy_from_slice(&flags.to_le_bytes());
    b[16..20].copy_from_slice(&fov_deg.to_le_bytes());
    b[20..24].copy_from_slice(&ortho_size.to_le_bytes());
    b[24..28].copy_from_slice(&near.to_le_bytes());
    b[28..32].copy_from_slice(&far.to_le_bytes());
    b[32..36].copy_from_slice(&tr.translation.x.to_le_bytes());
    b[36..40].copy_from_slice(&tr.translation.y.to_le_bytes());
    b[40..44].copy_from_slice(&tr.translation.z.to_le_bytes());
    b[44..48].copy_from_slice(&tr.rotation.x.to_le_bytes());
    b[48..52].copy_from_slice(&tr.rotation.y.to_le_bytes());
    b[52..56].copy_from_slice(&tr.rotation.z.to_le_bytes());
    b[56..60].copy_from_slice(&tr.rotation.w.to_le_bytes());

    let h = fnv1a64(&b);
    if h == st.last_cunning_hash {
        return;
    }
    let store = store.store.clone();
    *inflight_hash = Some(h);
    *task = Some(IoTaskPool::get().spawn(async move {
        let id = match store.insert_alloc(&b) {
            Ok(v) => v,
            Err(_) => return false,
        };
        store.set_latest(fnv1a64(K_CUNNING), id).is_ok()
    }));
}

fn pull_unity_splines(
    store: Option<Res<BridgeDbStore>>,
    st: Option<ResMut<BridgeDbState>>,
    mut node_graph_res: ResMut<crate::nodes::NodeGraphResource>,
    mut graph_changed_writer: MessageWriter<crate::GraphChanged>,
    time: Res<Time>,
    mut tick: Local<BridgeTick>,
) {
    let Some(store) = store else {
        return;
    };
    let Some(mut st) = st else {
        return;
    };
    if tick.timer.is_none() {
        tick.timer = Some(Timer::from_seconds(
            1.0 / BRIDGE_PULL_SPLINES_HZ,
            TimerMode::Repeating,
        ));
    }
    if !bridge_tick(&mut tick.timer, &time) {
        return;
    }

    // Snapshot spline nodes without holding the graph lock during DB I/O / decode.
    let spline_nodes: Vec<(uuid::Uuid, String, u32)> = {
        let root = &node_graph_res.0;
        root
            .nodes
            .iter()
            .filter_map(|(id, n)| {
                if !matches!(n.node_type, crate::nodes::NodeType::Spline) {
                    return None;
                }
                let key = n
                    .parameters
                    .iter()
                    .find(|p| p.name == "spline_blob_key")
                    .and_then(|p| match &p.value {
                        crate::nodes::parameter::ParameterValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if key.is_empty() {
                    return None;
                }
                let basis_id = n
                    .parameters
                    .iter()
                    .find(|p| p.name == "spline_source_basis")
                    .and_then(|p| match &p.value {
                        crate::nodes::parameter::ParameterValue::Int(v) => Some(*v as u32),
                        _ => None,
                    })
                    .unwrap_or(1);
                Some((*id, key, basis_id))
            })
            .collect()
    };

    let mut updates: Vec<(uuid::Uuid, crate::nodes::parameter::ParameterValue, u64)> = Vec::new();
    for (id, key, basis_id) in spline_nodes {
        let h = fnv1a64(key.as_bytes());
        let blob_id = store.store.get_latest(h).ok().unwrap_or(0);
        if blob_id == 0 {
            continue;
        }
        if st.last_spline_blob.get(&id).copied().unwrap_or(0) == blob_id {
            continue;
        }
        let bytes = match store.store.get(blob_id).ok().flatten() {
            Some(b) => b,
            None => continue,
        };
        let Some(container) = spline_snapshot_fbs::decode_snapshot_fbs_to_container(&bytes, basis_id) else {
            continue;
        };
        updates.push((
            id,
            crate::nodes::parameter::ParameterValue::UnitySpline(container),
            blob_id,
        ));
    }
    if updates.is_empty() {
        return;
    }

    let root = &mut node_graph_res.0;
    let mut changed = false;
    for (id, value, blob_id) in updates {
        let Some(n) = root.nodes.get_mut(&id) else {
            continue;
        };
        if let Some(p) = n.parameters.iter_mut().find(|p| p.name == "spline") {
            p.value = value;
            st.last_spline_blob.insert(id, blob_id);
            root.mark_dirty(id);
            changed = true;
        }
    }
    if changed {
        graph_changed_writer.write(crate::GraphChanged);
    }
}

pub struct BridgeDbSyncPlugin;
impl Plugin for BridgeDbSyncPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(crate::launcher::plugin::AppState::Editor),
            open_bridge_db_on_enter,
        );
        app.add_systems(
            Update,
            (
                pull_unity_viewport,
                pull_unity_splines,
                push_cunning_viewport,
            )
                .run_if(in_state(crate::launcher::plugin::AppState::Editor)),
        );
    }
}
