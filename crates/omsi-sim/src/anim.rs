//! `[newanim]` evaluation: turns script variables into per-mesh transforms.
//!
//! All maths happens in the model frame (x right, y forward, z up). `anim_rot` rotates about
//! the local **x** axis of the origin frame, `anim_trans` translates along it (doors use
//! `origin_rot_y -90` to turn that axis up, wheels spin about it directly, a blind slides along
//! it); `origin_*` commands move/rotate that frame, `origin_from_mesh` takes the mesh's stored
//! pivot matrix whose first row is that axis. Angles are used with the sign the file gives
//! them, and the blocks of one mesh are composed in file order (the first listed acts on the
//! mesh first), which with column vectors means the product is built from the right.

use glam::{Mat4, Quat, Vec3};
use omsi_model::{AnimKind, AnimOrigin, Animation, MeshDef};
use omsi_script::VarId;

/// Runtime state of one animation (smoothed value).
#[derive(Debug, Clone)]
pub struct AnimState {
    pub var: Option<VarId>,
    pub value: f32,
    pub initialized: bool,
}

/// Precomputed animation chain for one mesh.
#[derive(Debug, Clone)]
pub struct MeshAnimator {
    pub anims: Vec<(Animation, Mat4, AnimState)>,
    pub parent: Option<usize>,
}

/// Convert an o3d pivot matrix (D3D frame, column-vector form) to the model frame.
pub fn pivot_from_mesh(m: &omsi_o3d::Mesh) -> Mat4 {
    let s = Mat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    s * m.transform * s
}

pub fn origin_matrix(origins: &[AnimOrigin], pivot: Mat4) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    for o in origins {
        let op = match o {
            AnimOrigin::Trans(t) => Mat4::from_translation(Vec3::from_array(*t)),
            AnimOrigin::RotX(a) => Mat4::from_rotation_x(-a.to_radians()),
            AnimOrigin::RotY(a) => Mat4::from_rotation_y(-a.to_radians()),
            AnimOrigin::RotZ(a) => Mat4::from_rotation_z(-a.to_radians()),
            AnimOrigin::FromMesh => pivot,
        };
        m *= op;
    }
    m
}

impl MeshAnimator {
    pub fn new(def: &MeshDef, pivot: Mat4, resolve: impl Fn(&str) -> Option<VarId>) -> Self {
        let anims = def
            .animations
            .iter()
            .map(|a| {
                let var = resolve(&a.variable);
                if var.is_none() && !a.variable.is_empty() {
                    log::warn!("animation variable {} not found", a.variable);
                }
                (a.clone(), origin_matrix(&a.origins, pivot), AnimState { var, value: 0.0, initialized: false })
            })
            .collect();
        Self { anims, parent: None }
    }

    pub fn has_animations(&self) -> bool {
        !self.anims.is_empty() || self.parent.is_some()
    }

    /// Advance smoothing and return the mesh-space transform.
    pub fn update(&mut self, dt: f32, vars: &[f32]) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        for (a, origin, st) in &mut self.anims {
            let target = st.var.map(|v| vars[v as usize]).unwrap_or(0.0) * a.factor + a.offset;
            if !st.initialized {
                st.value = target;
                st.initialized = true;
            } else {
                let mut v = target;
                if a.delay > 0.0 {
                    // `delay` is a rate, not a time: the value closes the gap to its target
                    // at `delay` per second (the SD200's handbrake lever says `delay 10`,
                    // its doors 3 to 4). Read as a time constant it made the handbrake
                    // crawl for ten seconds.
                    let k = (dt * a.delay).min(1.0);
                    v = st.value + (target - st.value) * k;
                }
                if a.max_speed > 0.0 {
                    let max_step = a.max_speed * dt;
                    v = st.value + (v - st.value).clamp(-max_step, max_step);
                }
                st.value = v;
            }
            let local = match a.kind {
                Some(AnimKind::Rot) => Mat4::from_quat(Quat::from_rotation_x(-st.value.to_radians())),
                Some(AnimKind::Trans) => Mat4::from_translation(Vec3::new(st.value, 0.0, 0.0)),
                None => Mat4::IDENTITY,
            };
            // The original multiplies its matrices the Direct3D way round (a row vector
            // times the matrix), so a mesh with several [newanim] blocks is transformed by
            // the first one first and the later ones act on the result; written with column
            // vectors that is the reverse product. The SD200's wiper blade proves it: its
            // four blocks are its own two rotations about the arm's tip followed by the
            // arm's own two about the arm's base, and composed the other way round the
            // blade leaves the arm 1.2 m behind. Angles keep the same sign as the origin
            // rotations they are measured in (both are read in the original's left-handed
            // frame, so both change sign here) - with only one of them flipped the wiper
            // sweeps down into the bonnet instead of across the glass.
            m = *origin * local * origin.inverse() * m;
        }
        m
    }
}

/// `[animparent]`: a mesh hangs on the mesh whose `[mesh_ident]` it names and moves with
/// it (the NL202's changer, ticket printer and IBIS keys ride on the cash desk, which
/// swings with the driver's door). Sets the parent of every mesh, `defs` being the
/// meshes in the order the animators have them. A name nobody carries leaves the mesh
/// on its own, like the original. The parent is looked for among the meshes before this
/// one, the last of them that carries the name (Omsi.exe resolves `[animparent]` as it
/// reads the file, 0x5f1b3x: door variants that reuse their arms' names hung the later
/// variant's leaves on the first variant's arm, #348); one listed only after it is taken
/// as a last resort.
pub fn link_parents(animators: &mut [MeshAnimator], defs: &[&MeshDef]) {
    // `OMSI_NO_ANIMPARENT=1`: every mesh on its own, for an A/B
    if omsi_cfg::env::var_os("OMSI_NO_ANIMPARENT").is_some() {
        return;
    }
    for (i, a) in animators.iter_mut().enumerate() {
        a.parent = defs.get(i).and_then(|d| d.anim_parent.as_deref()).map(str::trim).filter(|n| !n.is_empty()).and_then(|name| {
            let named = |d: &&&MeshDef| d.mesh_ident.as_deref().map(|m| m.trim().eq_ignore_ascii_case(name)).unwrap_or(false);
            // (one only listed after it: taken all the same, as before)
            defs[..i.min(defs.len())].iter().rposition(|d| named(&d)).or_else(|| defs.iter().position(|d| named(&d)))
        }).filter(|p| *p != i);
    }
}

/// Put the parents' movement onto their children: a child's own animation happens in the
/// model frame first, then everything its parent chain does (the original multiplies the
/// child's matrix by the parent's, row-vector style). Parents may be listed after their
/// children and chains may be deep; a loop is cut where it closes.
pub fn apply_parents(animators: &[MeshAnimator], transforms: &mut [Mat4]) {
    if animators.iter().all(|a| a.parent.is_none()) {
        return;
    }
    let n = transforms.len().min(animators.len());
    let local: Vec<Mat4> = transforms[..n].to_vec();
    let mut done = vec![false; n];
    let mut chain: Vec<usize> = Vec::new();
    for start in 0..n {
        // walk up to the first mesh that is finished (or has no parent) ...
        chain.clear();
        let mut k = start;
        while !done[k] {
            if chain.contains(&k) {
                break;
            }
            chain.push(k);
            match animators[k].parent.filter(|p| *p < n) {
                Some(p) => k = p,
                None => break,
            }
        }
        // ... and come back down, each mesh taking its parent's final transform
        for &c in chain.iter().rev() {
            if done[c] {
                continue;
            }
            transforms[c] = match animators[c].parent.filter(|p| *p < n && done[*p]) {
                Some(p) => transforms[p] * local[c],
                None => local[c],
            };
            done[c] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animator(parent: Option<usize>) -> MeshAnimator {
        MeshAnimator { anims: Vec::new(), parent }
    }

    #[test]
    fn parents_compose_down_the_chain() {
        // 2 -> 0 -> 1 (the parent of 0 is listed after it)
        let mut a = vec![animator(Some(1)), animator(None), animator(Some(0))];
        let t0 = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let t1 = Mat4::from_rotation_z(0.5);
        let t2 = Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0));
        let mut xf = vec![t0, t1, t2];
        apply_parents(&a, &mut xf);
        assert!(xf[1].abs_diff_eq(t1, 1e-6));
        assert!(xf[0].abs_diff_eq(t1 * t0, 1e-6));
        assert!(xf[2].abs_diff_eq(t1 * t0 * t2, 1e-6));
        // a loop is cut instead of hanging
        a[1].parent = Some(2);
        let mut xf = vec![t0, t1, t2];
        apply_parents(&a, &mut xf);
        assert!(xf.iter().all(|m| m.is_finite()));
    }

    #[test]
    fn parents_found_by_mesh_ident() {
        let desk = MeshDef { mesh_ident: Some("zahltisch".into()), ..Default::default() };
        let key = MeshDef { anim_parent: Some("Zahltisch ".into()), ..Default::default() };
        let lost = MeshDef { anim_parent: Some("nobody".into()), ..Default::default() };
        let mut a = vec![animator(None), animator(None), animator(None)];
        link_parents(&mut a, &[&key, &desk, &lost]);
        assert_eq!(a.iter().map(|a| a.parent).collect::<Vec<_>>(), vec![Some(1), None, None]);
        // two door variants with the same arm names: each leaf hangs on the arm before it
        let arm_a = MeshDef { mesh_ident: Some("arm".into()), ..Default::default() };
        let leaf_a = MeshDef { anim_parent: Some("arm".into()), ..Default::default() };
        let arm_b = MeshDef { mesh_ident: Some("arm".into()), ..Default::default() };
        let leaf_b = MeshDef { anim_parent: Some("arm".into()), ..Default::default() };
        let mut a = vec![animator(None), animator(None), animator(None), animator(None)];
        link_parents(&mut a, &[&arm_a, &leaf_a, &arm_b, &leaf_b]);
        assert_eq!(a.iter().map(|a| a.parent).collect::<Vec<_>>(), vec![None, Some(0), None, Some(2)]);
    }
}
