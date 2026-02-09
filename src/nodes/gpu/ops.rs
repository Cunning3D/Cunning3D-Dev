use crate::libs::geometry::attrs;
use crate::nodes::parameter::{Parameter, ParameterValue};
use bevy::prelude::Vec3;

#[derive(Clone, Debug)]
pub enum GpuOp {
    AffineVec3 {
        domain: i32,
        attr: String,
        mul: Vec3,
        add: Vec3,
    },
}

#[inline]
pub fn fold_affine_chain(ops: &[GpuOp]) -> Option<GpuOp> {
    let mut dom = 0i32;
    let mut attr: Option<String> = None;
    let mut mul = Vec3::ONE;
    let mut add = Vec3::ZERO;
    for op in ops {
        let GpuOp::AffineVec3 {
            domain,
            attr: a,
            mul: m,
            add: ad,
        } = op;
        if let Some(ex) = &attr {
            if ex != a {
                return None;
            }
        } else {
            attr = Some(a.clone());
            dom = *domain;
        }
        if *domain != dom {
            return None;
        }
        // Compose: v' = (v * mul + add); then apply op: v'' = v' * m + ad => v'' = v * (mul*m) + (add*m + ad)
        mul *= *m;
        add = add * *m + *ad;
    }
    Some(GpuOp::AffineVec3 {
        domain: dom,
        attr: attr?,
        mul,
        add,
    })
}

#[inline]
pub fn lower_attribute_kernel(params: &[Parameter]) -> GpuOp {
    let mut domain = 0i32;
    let mut attr: &str = attrs::P;
    let mut op = 0i32;
    let mut value = Vec3::ZERO;
    for p in params {
        match (p.name.as_str(), &p.value) {
            ("domain", ParameterValue::Int(v)) => domain = *v,
            ("attr", ParameterValue::String(s)) => attr = s.as_str(),
            ("op", ParameterValue::Int(v)) => op = *v,
            ("value", ParameterValue::Vec3(v)) => value = *v,
            _ => {}
        }
    }
    let (mul, add) = match op {
        0 => (Vec3::ONE, value),  // Add
        1 => (value, Vec3::ZERO), // Mul
        _ => (Vec3::ZERO, value), // Set
    };
    GpuOp::AffineVec3 {
        domain,
        attr: attr.to_string(),
        mul,
        add,
    }
}
