use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{now_ms, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_agent_freeride.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Agent Free Ride, LEVEL 2 (`l2_level.w3d`). Level 1 has its own test; this one
// exists because level 2 is reported as heavily laggy and as emitting
// `[W3D-MISS]` warnings (a model whose mesh resource never reached the GPU, so
// it is silently not drawn).
//
// The level is chosen through `flash_start`'s first real argument — the same
// handler the menu SWF calls — so no Flash interaction is needed.
browser_e2e_test!(test_misc_agent_freeride_level2, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_startup_do();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    // Wait for the game movie to mount (wrapper -> gameloader -> agent_freeride).
    for _ in 0..40 {
        player.step_frames(20).await;
        if let Ok(vm_rust::director::static_datum::StaticDatum::String(s)) =
            player.eval_datum("string(gGame)").await
        {
            if !s.is_empty() { break; }
        }
    }

    let load_start = now_ms();
    let started = ask!("flash_start(0, \"2\", \"1\", \"0\", \"1\", \"0\", \"0\")");
    probe(format!("flash_start -> {} level={}", started, ask!("gGame.GetLevelId()")));

    // Level 2 streams a bigger track than level 1, so give the build more room.
    player.step_frames(600).await;
    player.step_frames(150).await;
    let load_ms = now_ms() - load_start;
    probe(format!("level2 loaded in {:.0} ms, frame={} status={}",
        load_ms, player.current_frame(), ask!("string(gGame.GetGameStatus())")));

    let snapshots = SnapshotContext::new("misc", "agent_freeride_l2");
    let _ = snapshots.verify("01_gameplay", player.snapshot_stage());

    probe(format!("level2 models={} worldModels={} tokens={} culling={}",
        ask!("gGame.Get3D().model.count"),
        ask!("string(gGame.GetCullingManager().pCullingList.count)"),
        ask!("gGame.GetTokenManager().GetTrackTokens().count"),
        ask!("string(gGame.GetCullingManager().pHorizontalBlockNum)")));

    // Frame cost during gameplay — the "very laggy" complaint. Timed in blocks so
    // one slow frame doesn't dominate, and reported per frame.
    for block in 0..4 {
        let t0 = now_ms();
        player.step_frames(30).await;
        let dt = now_ms() - t0;
        probe(format!("level2 frame cost block {}: {:.1} ms/frame over 30 frames",
            block, dt / 30.0));
    }

    // --- jetpack attachment ------------------------------------------------
    // Level 2's gadget is the jetpack (`Snowboard Race Player.ActivateGadgetByLevel`).
    // Its flames are `fx_flame_*_dyn` and `StartFlames` sets their LOCAL transform to
    // identity, so where they end up is decided entirely by their parent in the
    // scene graph — which is driven off the rider's bone world transforms.
    let _ = ask!("gGame.GetPlayers()[1].ActivateGadget(#jetpack)");
    player.step_frames(10).await;
    probe(format!("JET gadget={} gfxState={}",
        ask!("string(gGame.GetPlayers()[1].pActiveGadget)"),
        ask!("string(gGame.GetPlayers()[1].GetVehicleGfx().GetPlayerGfx().GetState())")));
    for m in ["fx_flame_1_dyn", "fx_flame_2_dyn"] {
        probe(format!("JET {} parent={} vis={} local={} world={}",
            m,
            ask!(&format!("string(gGame.Get3D().model(\"{}\").parent)", m)),
            ask!(&format!("string(gGame.Get3D().model(\"{}\").visibility)", m)),
            ask!(&format!("string(gGame.Get3D().model(\"{}\").transform.position)", m)),
            ask!(&format!("string(gGame.Get3D().model(\"{}\").getWorldTransform().position)", m))));
    }
    // The rider rig: root -> base -> skinned model, plus the spine bone the pack
    // should sit on.
    const G: &str = "gGame.GetPlayers()[1].GetVehicleGfx().GetPlayerGfx()";
    probe(format!("JET root={} base={} mdl={} mdlName={}",
        ask!(&format!("string({}.GetCharacterRoot().getWorldTransform().position)", G)),
        ask!(&format!("string({}.GetCharacterBase().getWorldTransform().position)", G)),
        ask!(&format!("string({}.GetCharacterMdl().getWorldTransform().position)", G)),
        ask!(&format!("{}.GetCharacterMdl().name", G))));
    // Find the jetpack model: enumerate the scene once and keep anything that
    // could be rider equipment, plus everything parented into the rider rig.
    let total = ask!("gGame.Get3D().model.count");
    let n_models: usize = total.trim_start_matches("Int(").trim_end_matches(')').parse().unwrap_or(0);
    let mut hits: Vec<String> = Vec::new();
    for i in 1..=n_models {
        if let Ok(vm_rust::director::static_datum::StaticDatum::String(nm)) =
            player.eval_datum(&format!("gGame.Get3D().model[{}].name", i)).await
        {
            let l = nm.to_lowercase();
            if l.contains("jet") || l.contains("pack") || l.contains("gad")
                || l.contains("zaino") || l.starts_with("veh_player")
                || l.contains("player_1")
            {
                hits.push(nm);
            }
        }
    }
    probe(format!("JET scene models={} candidates={:?}", n_models, hits));
    for g in ["GetCharacterRoot", "GetCharacterBase", "GetCharacterMdl"] {
        let c = ask!(&format!("{}.{}().child.count", G, g));
        let mut kids = Vec::new();
        for k in 1..=6 {
            let n = ask!(&format!("{}.{}().child[{}].name", G, g, k));
            if n.starts_with("<err") { break; }
            kids.push(n);
        }
        probe(format!("JET {} childCount={} kids={:?}", g, c, kids));
    }

    // The jetpack is a MESH of the skinned rider, not its own model, so per-mesh
    // shader/blend state and draw order decide how it layers against the body.
    probe(format!("JET rider meshes={} resource={} visibility={}",
        ask!(&format!("{}.GetCharacterMdl().shaderList.count", G)),
        ask!(&format!("{}.GetCharacterMdl().resource.name", G)),
        ask!(&format!("string({}.GetCharacterMdl().visibility)", G))));
    for i in 1..=10 {
        let nm = ask!(&format!("{}.GetCharacterMdl().shaderList[{}].name", G, i));
        if nm.starts_with("<err") { break; }
        probe(format!("JET mesh[{}] shader={} blend={} transparent={} tex={} blendFunc={}",
            i, nm,
            ask!(&format!("string({}.GetCharacterMdl().shaderList[{}].blend)", G, i)),
            ask!(&format!("string({}.GetCharacterMdl().shaderList[{}].transparent)", G, i)),
            ask!(&format!("string({}.GetCharacterMdl().shaderList[{}].textureList[1])", G, i)),
            ask!(&format!("string({}.GetCharacterMdl().shaderList[{}].blendFunctionList[1])", G, i))));
    }

    // Dump the rig so the jetpack's anchor bone can be identified by name.
    let bone_count = ask!(&format!("{}.GetCharacterMdl().resource.bone.count", G));
    probe(format!("JET boneCount={}", bone_count));
    for i in 1..=31 {
        let n = ask!(&format!("{}.GetCharacterMdl().bonesPlayer.bone[{}].name", G, i));
        if n.starts_with("<err") { break; }
        probe(format!("JET bone[{}] name={}", i, n));
    }
    for bone in ["Bip01 Spine1", "Bip01 Spine", "Bip01 Pelvis"] {
        let id = ask!(&format!("{}.GetCharacterMdl().resource.getBoneId(\"{}\")", G, bone));
        probe(format!("JET bone {} id={} world={}",
            bone, id,
            ask!(&format!("string({}.GetCharacterMdl().bonesPlayer.bone[{}.GetCharacterMdl().resource.getBoneId(\"{}\")].worldTransform.position)", G, G, bone))));
    }
    let _ = snapshots.verify("03_jetpack", player.snapshot_stage());

    let _ = snapshots.verify("02_ride", player.snapshot_stage());
    probe(format!("level2 end gdist={}",
        ask!("gGame.GetPlayerVehicle().GetVehicle().GetGroundDistance()")));

    Ok(())
});
