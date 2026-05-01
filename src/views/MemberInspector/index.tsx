import { useState } from "react";
import { ICastMemberRef } from "dirplayer-js-api";
import PreviewCanvas from "../../components/PreviewCanvas";
import ScriptMemberPreview from "../../components/ScriptMemberPreview";
import { useAppSelector, useMemberSnapshot } from "../../store/hooks";
import { ICastMemberIdentifier, memberRefEqualsSafe } from "../../vm";
import styles from "./styles.module.css";
import { player_print_member_bitmap_hex } from 'vm-rust'
import FilmLoopInspector from "../FilmLoopInspector";

interface IMemberInspectorProps {
  memberId: ICastMemberIdentifier;
}

interface ITextMemberPreviewProps {
  text: string;
}

const normalizeLineEndings = (str: string, normalized = "\r\n") =>
  str.replace(/\r?\n|\r/g, normalized);

function TextMemberPreview({ text }: ITextMemberPreviewProps) {
  return <p className={styles.textPreview}>{normalizeLineEndings(text)}</p>;
}

function FontPreview() {
  const [fontSize, setFontSize] = useState(12);
  return (
    <div>
      <label>
        Font size:{" "}
        <input
          type="number"
          min={4}
          max={72}
          value={fontSize}
          onChange={(e) => setFontSize(Number(e.target.value))}
          style={{ width: 50 }}
        />
      </label>
      <PreviewCanvas fontSize={fontSize} />
    </div>
  );
}

export default function MemberInspector({ memberId }: IMemberInspectorProps) {
  const memberSnapshot = useMemberSnapshot(memberId);
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
      #{memberSnapshot?.number} {memberSnapshot?.type}: {memberSnapshot?.name}
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
          />
        )}
        {memberSnapshot?.type === "bitmap" && (
          <div>
            <p>{memberSnapshot.width}x{memberSnapshot.height}x{memberSnapshot.bitDepth}</p>
            <p>Reg point: {memberSnapshot.regX}x{memberSnapshot.regY}</p>
            <p>Palette ref: {memberSnapshot.paletteRef}</p>
            <button onClick={() => player_print_member_bitmap_hex(memberId.castNumber, memberId.memberNumber)}>Print hex</button>
            <PreviewCanvas />
          </div>)}
        {memberSnapshot?.type === "filmLoop" && (
          <FilmLoopInspector memberId={memberId} />
        )}
        {memberSnapshot?.type === "font" && (
          <FontPreview />
        )}
        {memberSnapshot?.type === "flash" && (
          <div>
            <p>{memberSnapshot.width}x{memberSnapshot.height}</p>
            <p>Reg point: {memberSnapshot.regX}x{memberSnapshot.regY}</p>
            <p>Data size: {memberSnapshot.dataSize} bytes</p>
            {memberSnapshot.directToStage !== undefined && <p>Direct to stage: {String(memberSnapshot.directToStage)}</p>}
            {memberSnapshot.sourceFileName && <p>Source: {memberSnapshot.sourceFileName}</p>}
            {memberSnapshot.quality && <p>Quality: {memberSnapshot.quality}</p>}
            {memberSnapshot.scaleMode && <p>Scale mode: {memberSnapshot.scaleMode}</p>}
            {memberSnapshot.playbackMode && <p>Playback: {memberSnapshot.playbackMode}</p>}
          </div>
        )}
        {memberSnapshot?.type === "shockwave3d" && (
          <div>
            <p>{memberSnapshot.width}x{memberSnapshot.height}</p>
            <p>Reg point: {memberSnapshot.regX}x{memberSnapshot.regY}</p>
          </div>
        )}
        {memberSnapshot?.type === "palette" && <div>
          Ref id: {memberSnapshot.paletteRef}
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
