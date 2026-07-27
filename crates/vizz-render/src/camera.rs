//! Camera and the room it looks into.
//!
//! Until now the camera was four hardcoded lines in the vertex shader: a
//! fixed elevation, a fixed distance, and a divide by view depth. That is
//! enough to see a shape, and not enough for anything that depends on
//! *where you are* — parallax, forced perspective, depth of field. All
//! three need a real view/projection.
//!
//! The room is why the projection has to be honest rather than merely
//! plausible. A box whose front face lines up exactly with the frame edges
//! reads as a window rather than as a box on screen, and that illusion
//! collapses immediately if the field of view and the box dimensions
//! disagree even slightly — so the two are derived from each other here
//! rather than tuned by eye.

use glam::{Mat4, Vec3};

/// Where the camera is and what it is doing. Angles in radians.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Distance from the origin.
    pub distance: f32,
    /// Rotation around Y.
    pub orbit: f32,
    /// Rotation above the horizon.
    pub elevation: f32,
    /// Vertical field of view.
    pub fov: f32,
    pub aspect: f32,
    /// Distance at which things are sharp, for depth of field.
    pub focus: f32,
    /// How quickly sharpness falls off either side of `focus`.
    pub defocus: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            distance: 3.5,
            orbit: 0.0,
            // The old hardcoded downward tilt. Without some elevation the
            // grid mode sits exactly edge-on and vanishes, and volumetric
            // shapes lose most of their depth cues.
            elevation: 0.34,
            fov: 0.9,
            aspect: 16.0 / 9.0,
            focus: 3.5,
            defocus: 0.0,
        }
    }
}

/// Everything the shaders need, derived once per frame on the CPU.
#[derive(Debug, Clone, Copy)]
pub struct CameraUniforms {
    pub view_proj: [[f32; 4]; 4],
    /// Camera basis, so sprites can face the camera without each vertex
    /// re-deriving it from the matrix.
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub position: [f32; 3],
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (se, ce) = self.elevation.sin_cos();
        let (so, co) = self.orbit.sin_cos();
        Vec3::new(ce * so, se, ce * co) * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), Vec3::ZERO, Vec3::Y)
    }

    pub fn projection(&self) -> Mat4 {
        // `perspective_rh`, not `perspective_rh_gl`: wgpu's clip space has
        // depth 0..1 like Metal and D3D, and the GL variant's -1..1 would
        // put half of everything behind the near plane.
        Mat4::perspective_rh(
            self.fov.clamp(0.1, 2.8),
            self.aspect.max(0.01),
            // Near plane close enough to fly into the cloud without
            // clipping it away, far enough to keep depth precision usable.
            0.05,
            200.0,
        )
    }

    pub fn uniforms(&self) -> CameraUniforms {
        let view = self.view();
        // The view matrix's rows are the camera basis in world space, which
        // is exactly what billboarding needs — no separate derivation, and
        // no chance of the two disagreeing.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        CameraUniforms {
            view_proj: (self.projection() * view).to_cols_array_2d(),
            right: right.to_array(),
            up: up.to_array(),
            position: self.eye().to_array(),
        }
    }

    /// Half-extents of a room whose front face exactly fills the frame at
    /// `depth` in front of the camera.
    ///
    /// This is the forced-perspective trick: get it right and the frame
    /// edge *is* the room edge, so the screen reads as a window. Guessing
    /// the numbers instead leaves a sliver of background visible along one
    /// edge, which reads as a floating box and gives the illusion away.
    pub fn frustum_half_extents(&self, depth: f32) -> (f32, f32) {
        let h = (self.fov.clamp(0.1, 2.8) * 0.5).tan() * depth;
        (h * self.aspect.max(0.01), h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4Swizzles;

    fn project(cam: &Camera, p: Vec3) -> Vec3 {
        let clip = Mat4::from_cols_array_2d(&cam.uniforms().view_proj) * p.extend(1.0);
        // Perspective divide into normalised device coordinates.
        (clip.xyz() / clip.w).into()
    }

    /// The origin must land in the middle of the frame at any orbit, or
    /// the camera is not actually looking at what it claims to.
    #[test]
    fn the_camera_looks_at_the_origin() {
        for orbit in [0.0, 1.0, 3.0, -2.2] {
            let cam = Camera { orbit, ..Default::default() };
            let ndc = project(&cam, Vec3::ZERO);
            assert!(
                ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4,
                "orbit {orbit}: origin projected to {ndc:?}"
            );
            // And in front of the camera, not behind it.
            assert!((0.0..1.0).contains(&ndc.z), "origin depth {} out of range", ndc.z);
        }
    }

    /// wgpu clip space is 0..1 in depth. Using the OpenGL projection here
    /// is an easy mistake and puts half the scene behind the near plane,
    /// so assert the convention rather than assuming it.
    #[test]
    fn depth_is_in_wgpu_clip_range() {
        let cam = Camera::default();
        // Comfortably inside the frustum at both ends: sampling closer
        // than the 0.05 near plane clips and reports a negative depth,
        // which says nothing about the convention.
        let near = project(&cam, cam.eye() * 0.5);
        let far = project(&cam, -cam.eye() * 20.0);
        assert!(near.z >= 0.0 && near.z <= 1.0, "near depth {near:?}");
        assert!(far.z >= 0.0 && far.z <= 1.0, "far depth {far:?}");
        // Closer must be nearer in depth than further away.
        assert!(near.z < far.z, "depth is inverted: {} vs {}", near.z, far.z);
    }

    /// The forced-perspective property, stated as a test: a room built
    /// from `frustum_half_extents` must have its front corners land exactly
    /// on the frame corners. If this drifts, the illusion shows a sliver of
    /// background along an edge and the room reads as a floating box.
    #[test]
    fn room_front_face_fills_the_frame_exactly() {
        for (fov, aspect) in [(0.9, 16.0 / 9.0), (0.6, 4.0 / 3.0), (1.4, 2.35)] {
            let cam = Camera { fov, aspect, orbit: 0.0, elevation: 0.0, ..Default::default() };
            let depth = 2.0;
            let (hx, hy) = cam.frustum_half_extents(depth);
            // A point on the front face, in front of the camera by `depth`.
            let z = cam.distance - depth;
            for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let ndc = project(&cam, Vec3::new(hx * sx, hy * sy, z));
                assert!(
                    (ndc.x.abs() - 1.0).abs() < 1e-3 && (ndc.y.abs() - 1.0).abs() < 1e-3,
                    "fov {fov} aspect {aspect}: corner landed at {ndc:?}, not the frame edge"
                );
            }
        }
    }

    /// Orbiting must move the viewpoint rather than spinning the world in
    /// place — that difference is the whole of parallax. A point nearer the
    /// camera has to shift across the frame more than a distant one.
    #[test]
    fn orbiting_produces_parallax() {
        let a = Camera { orbit: 0.0, elevation: 0.0, ..Default::default() };
        let b = Camera { orbit: 0.15, elevation: 0.0, ..Default::default() };
        let near = Vec3::new(0.0, 0.0, 1.5);
        let far = Vec3::new(0.0, 0.0, -1.5);
        let near_shift = (project(&b, near).x - project(&a, near).x).abs();
        let far_shift = (project(&b, far).x - project(&a, far).x).abs();
        assert!(
            near_shift > far_shift * 1.5,
            "no parallax: near moved {near_shift}, far moved {far_shift}"
        );
    }

    /// Field of view is the zoom control, so narrowing it must magnify.
    #[test]
    fn a_narrower_fov_magnifies() {
        let wide = Camera { fov: 1.2, elevation: 0.0, ..Default::default() };
        let tight = Camera { fov: 0.5, elevation: 0.0, ..Default::default() };
        let p = Vec3::new(0.4, 0.0, 0.0);
        assert!(
            project(&tight, p).x.abs() > project(&wide, p).x.abs(),
            "narrowing the field of view did not magnify"
        );
    }

    /// Billboards face the camera, so the basis must stay orthonormal at
    /// every orientation or sprites shear.
    #[test]
    fn the_camera_basis_stays_orthonormal() {
        for (orbit, elevation) in [(0.0, 0.0), (1.1, 0.34), (-2.0, -1.2), (3.0, 1.4)] {
            let u = Camera { orbit, elevation, ..Default::default() }.uniforms();
            let (r, up) = (Vec3::from(u.right), Vec3::from(u.up));
            assert!((r.length() - 1.0).abs() < 1e-4, "right not unit: {}", r.length());
            assert!((up.length() - 1.0).abs() < 1e-4, "up not unit: {}", up.length());
            assert!(r.dot(up).abs() < 1e-4, "basis not perpendicular: {}", r.dot(up));
        }
    }
}
