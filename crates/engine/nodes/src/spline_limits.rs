//! Bounds for the additional spline types and finite output checks for their consumers.

use crate::curves::CurveType;
use crate::field::Scalar;
use crate::geometry::{Attributes, Geometry};

fn additional(kind: CurveType) -> bool {
    matches!(kind, CurveType::CatmullRom | CurveType::Nurbs)
}

pub(crate) fn contains_new_splines(geometry: &Geometry) -> bool {
    geometry.curves.as_ref().is_some_and(|curves| curves.kinds.iter().copied().any(additional))
        || geometry.instances.as_ref().is_some_and(|instances| instances.items.iter().any(|item| contains_new_splines(&item.geometry)))
}

fn attributes_finite(attributes: &Attributes) -> bool {
    attributes.values().all(|attribute| {
        attribute.values.iter().all(|value| match value {
            Scalar::Float(value) => value.is_finite(),
            Scalar::Vector(value) => value.is_finite(),
            Scalar::Int(_) | Scalar::Bool(_) => true,
        })
    })
}

/// A linear check of produced geometry. The tighter coordinate bound applies only to the
/// new spline types; converted meshes, points and other curves must remain finite.
pub(crate) fn validate(geometry: &Geometry) -> Result<(), String> {
    let nonfinite = || "the spline operation produced non-finite geometry".to_string();
    if let Some(mesh) = &geometry.mesh
        && (!mesh.mesh.positions.iter().all(|point| point.is_finite()) || !attributes_finite(&mesh.attributes))
    {
        return Err(nonfinite());
    }
    if let Some(points) = &geometry.points
        && (!points.positions.iter().all(|point| point.is_finite())
            || !points.radius.iter().all(|value| value.is_finite())
            || !attributes_finite(&points.attributes))
    {
        return Err(nonfinite());
    }
    if let Some(curves) = &geometry.curves {
        if !curves.positions.iter().chain(&curves.handle_left).chain(&curves.handle_right).all(|point| point.is_finite())
            || !curves.radius.iter().chain(&curves.tilt).all(|value| value.is_finite())
            || !attributes_finite(&curves.attributes)
        {
            return Err(nonfinite());
        }
        for (curve, kind) in curves.kinds.iter().enumerate() {
            if !additional(*kind) {
                continue;
            }
            for at in curves.range(curve) {
                if [&curves.positions, &curves.handle_left, &curves.handle_right]
                    .into_iter()
                    .any(|values| values.get(at).is_some_and(|point| point.abs().max_element() > 1.0e12))
                    || [&curves.radius, &curves.tilt].into_iter().any(|values| values.get(at).is_some_and(|value| value.abs() > 1.0e12))
                {
                    return Err("Catmull Rom and NURBS coordinates, radius and tilt are limited to magnitude 1000000000000".into());
                }
                if *kind == CurveType::Nurbs
                    && curves.nurbs_weight.get(at).is_some_and(|weight| !weight.is_finite() || !(0.0..=1.0e8).contains(weight))
                {
                    return Err("NURBS weight must be finite and between zero and 100000000".into());
                }
            }
        }
    }
    if let Some(instances) = &geometry.instances {
        if !attributes_finite(&instances.attributes) {
            return Err(nonfinite());
        }
        for instance in &instances.items {
            if !instance.transform.is_finite() {
                return Err(nonfinite());
            }
            validate(&instance.geometry)?;
        }
    }
    Ok(())
}
