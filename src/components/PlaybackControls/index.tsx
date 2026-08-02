import { useState } from 'react'
import { faPlay, faStop, faRotateBack, faStopwatch, faCircleDot } from '@fortawesome/free-solid-svg-icons'
import IconButton from '../IconButton'
import styles from './styles.module.css'
import { play, stop, reset, start_profiling_recording, stop_profiling_recording, export_profiling_speedscope } from 'vm-rust'
import { isElectron } from '../../utils/electron'
import { isMcpEnabled, setMcpEnabled, getMcpPort, setMcpPort, getMcpUrl } from '../../mcp'

// Save the recorded speedscope profile to a downloadable file. Open the result
// at https://www.speedscope.app/ (or `speedscope <file>`).
function downloadSpeedscopeProfile() {
  const json = export_profiling_speedscope();
  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  const ts = new Date().toISOString().replace(/[:.]/g, '-').replace('T', '_').slice(0, 19);
  a.download = `dirplayer-${ts}.speedscope.json`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

function ProfileToggle() {
  const [recording, setRecording] = useState(false);
  return (
    <IconButton
      icon={recording ? faCircleDot : faStopwatch}
      onClick={() => {
        if (!recording) {
          start_profiling_recording();
          setRecording(true);
        } else {
          stop_profiling_recording();
          downloadSpeedscopeProfile();
          setRecording(false);
        }
      }}
      title={recording
        ? 'Recording Lingo VM profile — click to stop and download .speedscope.json'
        : 'Record a Lingo VM profile (open in speedscope)'}
    />
  );
}

function McpToggle() {
  const [enabled, setEnabled] = useState(() => isMcpEnabled());
  const [port, setPort] = useState(() => getMcpPort());
  const [editingPort, setEditingPort] = useState(false);
  const [portDraft, setPortDraft] = useState(String(port));

  const commitPort = () => {
    const parsed = parseInt(portDraft, 10);
    if (parsed > 0 && parsed <= 65535) {
      setPort(parsed);
      setMcpPort(parsed);
    } else {
      setPortDraft(String(port));
    }
    setEditingPort(false);
  };

  return (
    <div className={styles.mcpContainer}>
      <button
        className={enabled ? styles.mcpButtonActive : styles.mcpButton}
        onClick={() => {
          const next = !enabled;
          setEnabled(next);
          setMcpEnabled(next);
        }}
        title={enabled ? 'MCP server is running. Click to stop.' : 'Start MCP server for AI debugging tools'}
      >
        MCP {enabled ? 'ON' : 'OFF'}
      </button>
      {enabled && (
        editingPort ? (
          <input
            className={styles.mcpPortInput}
            value={portDraft}
            onChange={(e) => setPortDraft(e.target.value)}
            onBlur={commitPort}
            onKeyDown={(e) => { if (e.key === 'Enter') commitPort(); if (e.key === 'Escape') { setPortDraft(String(port)); setEditingPort(false); } }}
            autoFocus
            size={5}
          />
        ) : (
          <span
            className={styles.mcpUrl}
            onClick={() => { setPortDraft(String(port)); setEditingPort(true); }}
            title="Click to change port"
          >
            {getMcpUrl()}
          </span>
        )
      )}
    </div>
  );
}

export default function PlaybackControls() {
  return <div className={styles.container}>
    <IconButton icon={faPlay} onClick={() => { play() }} />
    <IconButton icon={faStop} onClick={() => { stop() }} />
    <IconButton icon={faRotateBack} onClick={() => { reset() }} />
    <ProfileToggle />
    {isElectron() && <>
      <div className={styles.spacer} />
      <McpToggle />
    </>}
  </div>
}
