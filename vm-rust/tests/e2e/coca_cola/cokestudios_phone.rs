use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{
    SnapshotContext, SnapshotOutput, TestConfig, TestHarness, datum,
};

const CONFIG: &str = include_str!("../configs/cc_cokestudios-scaled.toml");

/// The handset this test stands in for, as the page sees it: the CSS viewport
/// the browser reports (`innerWidth`/`innerHeight` in landscape, fullscreen)
/// and the physical pixels it puts behind each of those
/// (`devicePixelRatio`). Change these three and the whole test follows.
const PHONE_CSS: (u32, u32) = (873, 393);
const PHONE_DPR: f64 = 2.75;

/// A movie-space rect in the device pixels of the letterboxed `meet` layout,
/// so a crop keeps naming the thing it is cropping when the handset changes.
fn device_rect(l: f64, t: f64, r: f64, b: f64) -> (i32, i32, i32, i32) {
    let (cw, ch) = (
        PHONE_CSS.0 as f64 * PHONE_DPR,
        PHONE_CSS.1 as f64 * PHONE_DPR,
    );
    let (mw, mh) = (760.0, 520.0);
    let scale = f64::min(cw / mw, ch / mh);
    let (ox, oy) = ((cw - mw * scale) / 2.0, (ch - mh * scale) / 2.0);
    (
        (ox + l * scale) as i32,
        (oy + t * scale) as i32,
        (ox + r * scale) as i32,
        (oy + b * scale) as i32,
    )
}

/// Coke Studios on a PHONE-sized stage: 873x393 CSS pixels behind a 2400x1080
/// screen, the landscape viewport of a current handset, fullscreen.
///
/// The navigator's room list is the case that matters. It is not a text sprite
/// the renderer re-rasterises every frame but a bitmap the movie COMPOSES
/// itself (`ParentScript 24 - roomlist script` blits `member("roomlist").image`
/// plus the bullet art into `pScrollImg` and assigns it to
/// `member("roomDisplay").image`), so whatever resolution it is baked at is all
/// the renderer has to work with.
///
/// In CSS pixels alone `meet` picks min(873/760, 393/520) = 0.756 and the list
/// is baked MINIFIED — then the compositor magnifies the whole canvas 2.75x
/// back to the physical screen, which is the illegible mush a phone actually
/// showed. Reporting the pixel ratio makes the same phone a 2.077x
/// MAGNIFICATION (2401/760 vs 1081/520), which is the case the rest of the
/// pipeline — including the hi-res twin that keeps a composed bitmap sharp —
/// already handles.
browser_e2e_test!(test_cokestudios_phone, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_flash_config();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "cokestudios_phone");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    vm_rust::set_stage_size(PHONE_CSS.0, PHONE_CSS.1);
    vm_rust::set_stage_pixel_ratio(PHONE_DPR);

    player
        .step_until(datum("ilk(oRoom)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await
        .map_err(|e| format!("{e}\nNever entered the lobby (see cc_cokestudios.toml)."))?;
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(63)))
        .timeout(120.0)
        .await?;
    player.step_frames(150).await;

    // The canvas is sized in DEVICE pixels, not the CSS box.
    let want = (
        (PHONE_CSS.0 as f64 * PHONE_DPR).round() as u32,
        (PHONE_CSS.1 as f64 * PHONE_DPR).round() as u32,
    );
    if let SnapshotOutput::Rgba { width, height, .. } = player.snapshot_stage().to_rgba() {
        assert_eq!(
            (width, height),
            want,
            "the stage canvas is still being rendered at CSS-pixel size; set_stage_pixel_ratio did not reach the layout"
        );
    }

    snapshots.verify_with_ratio("lobby", player.snapshot_stage(), 0.25)?;
    // The room list alone — its movie-space box, in the device pixels the
    // letterboxed layout actually puts it at.
    let (l, t, r, b) = device_rect(438.0, 182.0, 718.0, 294.0);
    snapshots.verify_with_ratio("roomlist", player.snapshot_stage().crop(l, t, r, b), 0.25)?;

    Ok(())
});
