import { useState } from "react";
import { ICastMemberRef } from "dirplayer-js-api";
import PreviewCanvas from "../../components/PreviewCanvas";
import ScriptMemberPreview from "../../components/ScriptMemberPreview";
import { useAppSelector, useMemberSnapshot } from "../../store/hooks";
import { ICastMemberIdentifier, ISoundMemberSnapshot, memberRefEqualsSafe } from "../../vm";
import styles from "./styles.module.css";
import { player_print_member_bitmap_hex, player_play_member_sound, player_print_member_sound_hex } from 'vm-rust'
import FilmLoopInspector from "../FilmLoopInspector";
import { useTheme } from "../../utils/theme";

interface IMemberInspectorProps {
  memberId: ICastMemberIdentifier;
}

interface ITextMemberPreviewProps {
  text: string;
}

/** Key/value list shared by every member type's detail panel. */
function Facts({ children }: { children: React.ReactNode }) {
  return <div className={styles.facts}>{children}</div>;
}

function Fact({ label, value }: { label: string; value: React.ReactNode }) {
  if (value === undefined || value === null || value === '') return null;
  return (
    <>
      <div className={styles.factLabel}>{label}</div>
      <div className={styles.factValue}>{value}</div>
    </>
  );
}

const normalizeLineEndings = (str: string, normalized = "\r\n") =>
  str.replace(/\r?\n|\r/g, normalized);

function TextMemberPreview({ text }: ITextMemberPreviewProps) {
  return <p className={styles.textPreview}>{normalizeLineEndings(text)}</p>;
}

function SoundMemberPreview({
  memberId,
  snapshot,
}: {
  memberId: ICastMemberIdentifier;
  snapshot: ISoundMemberSnapshot;
}) {
  const durationSec = snapshot.duration ? (snapshot.duration / 1000).toFixed(2) : "?";
  return (
    <div className={styles.detail}>
      <Facts>
        <Fact label="Format" value={`${snapshot.sampleRate} Hz · ${snapshot.channels} ch · ${snapshot.bitsPerSample}-bit`} />
        <Fact label="Samples" value={snapshot.sampleCount} />
        <Fact label="Duration" value={`${durationSec}s`} />
        <Fact label="Loop" value={String(snapshot.loop)} />
        <Fact label="Codec" value={snapshot.codec || "?"} />
        <Fact label="Data size" value={`${snapshot.dataSize} bytes`} />
      </Facts>
      <div className={styles.actions}>
        <button
          className={styles.actionButton}
          onClick={() =>
            player_play_member_sound(memberId.castNumber, memberId.memberNumber)
          }
        >
          ▶ Play
        </button>
        <button
          className={styles.actionButton}
          onClick={() =>
            player_print_member_sound_hex(memberId.castNumber, memberId.memberNumber)
          }
        >
          Print raw bytes
        </button>
      </div>
    </div>
  );
}

function FontPreview() {
  const [fontSize, setFontSize] = useState(12);
  return (
    <div className={styles.detail}>
      <div className={styles.actions}>
        <label className={styles.fieldLabel}>
          Font size
          <input
            className={styles.numberInput}
            type="number"
            min={4}
            max={72}
            value={fontSize}
            onChange={(e) => setFontSize(Number(e.target.value))}
          />
        </label>
      </div>
      <PreviewCanvas fontSize={fontSize} />
    </div>
  );
}

export default function MemberInspector({ memberId }: IMemberInspectorProps) {
  const memberSnapshot = useMemberSnapshot(memberId);
  // The script viewer ships its own VS Code-style light and dark palettes;
  // point it at whichever one the app is currently in.
  const { resolved: theme } = useTheme();
  const scopes = useAppSelector((state) => state.vm.scopes);
  const currentScope = scopes.at(scopes.length - 1);
  const isScriptExecuting = memberRefEqualsSafe(
    memberId,
    currentScope?.script_member_ref
  );
  const bgScopes: [string, number, ICastMemberRef][] = scopes.slice(0, scopes.length - 1).map((scope) => [scope.handler_name, scope.bytecode_index, scope.script_member_ref]);

  if (!memberSnapshot) {
    return <div className={styles.container}>Loading {JSON.stringify(memberId)}...</div>;
  }

  return (
    <div className={styles.container}>
      <header className={styles.memberHeader}>
        <span className={styles.memberNumber}>#{memberSnapshot.number}</span>
        <span className={styles.memberName}>
          {memberSnapshot.name || <span className={styles.memberUnnamed}>untitled</span>}
        </span>
        <span className={styles.memberType}>{memberSnapshot.type}</span>
      </header>
      <div className={styles.preview}>
        {memberSnapshot?.type === "field" && (
          <TextMemberPreview text={memberSnapshot?.text || ''} />
        )}
        {memberSnapshot?.type === "script" && (
          <ScriptMemberPreview
            snapshot={memberSnapshot}
            highlightedBytecodeIndex={
              isScriptExecuting ? currentScope?.bytecode_index : undefined
            }
            highlightedHandlerName={
              isScriptExecuting ? currentScope?.handler_name : undefined
            }
            backgroundScopes={bgScopes}
            memberId={memberId}
            theme={theme}
          />
        )}
        {memberSnapshot?.type === "bitmap" && (
          <div className={styles.detail}>
            <Facts>
              <Fact label="Size" value={`${memberSnapshot.width} × ${memberSnapshot.height}`} />
              <Fact label="Bit depth" value={`${memberSnapshot.bitDepth}-bit`} />
              <Fact label="Reg point" value={`${memberSnapshot.regX}, ${memberSnapshot.regY}`} />
              <Fact label="Palette" value={memberSnapshot.paletteRef} />
            </Facts>
            <div className={styles.actions}>
              <button
                className={styles.actionButton}
                onClick={() => player_print_member_bitmap_hex(memberId.castNumber, memberId.memberNumber)}
              >
                Print hex
              </button>
            </div>
            <PreviewCanvas />
          </div>)}
        {memberSnapshot?.type === "filmLoop" && (
          <FilmLoopInspector memberId={memberId} />
        )}
        {memberSnapshot?.type === "font" && (
          <FontPreview />
        )}
        {memberSnapshot?.type === "sound" && (
          <SoundMemberPreview memberId={memberId} snapshot={memberSnapshot} />
        )}
        {memberSnapshot?.type === "flash" && (
          <div className={styles.detail}>
            <Facts>
              <Fact label="Size" value={`${memberSnapshot.width} × ${memberSnapshot.height}`} />
              <Fact label="Reg point" value={`${memberSnapshot.regX}, ${memberSnapshot.regY}`} />
              <Fact label="Data size" value={`${memberSnapshot.dataSize} bytes`} />
              <Fact
                label="Direct to stage"
                value={memberSnapshot.directToStage === undefined ? undefined : String(memberSnapshot.directToStage)}
              />
              <Fact label="Source" value={memberSnapshot.sourceFileName} />
              <Fact label="Quality" value={memberSnapshot.quality} />
              <Fact label="Scale mode" value={memberSnapshot.scaleMode} />
              <Fact label="Playback" value={memberSnapshot.playbackMode} />
            </Facts>
          </div>
        )}
        {memberSnapshot?.type === "shockwave3d" && (
          <div className={styles.detail}>
            <Facts>
              <Fact label="Size" value={`${memberSnapshot.width} × ${memberSnapshot.height}`} />
              <Fact label="Reg point" value={`${memberSnapshot.regX}, ${memberSnapshot.regY}`} />
            </Facts>
          </div>
        )}
        {memberSnapshot?.type === "palette" && <div className={styles.detail}>
          <Facts>
            <Fact label="Ref id" value={memberSnapshot.paletteRef} />
            <Fact label="Colors" value={memberSnapshot.colors?.length} />
          </Facts>
          {memberSnapshot.colors && <div className={styles.paletteGrid}>
            {memberSnapshot.colors.map((color, i) => (
              <div key={i} style={{ backgroundColor: `rgb(${color[0]}, ${color[1]}, ${color[2]})`, width: 20, height: 20 }} />
            ))}
          </div>}
        </div>}
      </div>
    </div>
  );
}
