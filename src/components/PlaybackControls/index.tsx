import { useState } from 'react'
import { faPlay, faStop, faBackwardStep } from '@fortawesome/free-solid-svg-icons'
import IconButton from '../IconButton'
import styles from './styles.module.css'
import { play, stop, rewind } from 'vm-rust'
import { isElectron } from '../../utils/electron'
import { isMcpEnabled, setMcpEnabled, getMcpPort, setMcpPort, getMcpUrl } from '../../mcp'
import { useAppSelector } from '../../store/hooks'

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
  const isPlaying = useAppSelector(state => state.vm.isPlaying);

  return <div className={styles.container}>
    <IconButton icon={faBackwardStep} onClick={() => { rewind() }} title="Rewind" />
    <IconButton icon={faStop} onClick={() => { stop() }} active={!isPlaying} title="Stop" />
    <IconButton icon={faPlay} onClick={() => { play() }} active={isPlaying} title="Play" />
    {isElectron() && <>
      <div className={styles.spacer} />
      <McpToggle />
    </>}
  </div>
}
