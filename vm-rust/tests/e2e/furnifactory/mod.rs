use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, datum, sprite};

const CONFIG: &str = include_str!("../configs/furnifactory.toml");

browser_e2e_test!(test_furnifactory_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "load");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player
        .step_until(sprite().member("alertbox2_start-up").visible(1.0))
        .timeout(10.0)
        .await?;
    snapshots.verify("init", player.snapshot_stage())?;

    player
        .click_sprite(sprite().member_prefix("alertbox2_start-up"))
        .await?;
    player
        .step_until(datum("ilk(oComputer)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(10.0)
        .await?;
    player
        .step_until(
            datum("not oComputer.oTimer.bPaused and oComputer.oTimer.iTime < 57")
                .equals(StaticDatum::Int(1)),
        )
        .timeout(10.0)
        .await?;
    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
