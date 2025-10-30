import { useEffect, useRef, useState } from 'react';
import styles from './styles.module.css';
import { eval_command } from "vm-rust";
import { useAppSelector } from '../../store/hooks';
import { selectDebugMessages } from '../../store/vmSlice';

export default function MessageInspector() {
  const [command, setCommand] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const debugMessages = useAppSelector(({ vm }) => selectDebugMessages(vm)).join('\n');
  const messageLogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    messageLogRef.current?.scrollTo(0, messageLogRef.current.scrollHeight);
  }, [debugMessages]);

  const handleEvaluate = () => {
    try {
      if (command.trim()) {
        eval_command(command);
        setHistory(prev => [...prev, command]);
        setHistoryIndex(-1);
      }
      setCommand('');
    } catch (err) {
      console.error(err);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleEvaluate();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (history.length > 0) {
        const newIndex = historyIndex === -1 ? history.length - 1 : Math.max(0, historyIndex - 1);
        setHistoryIndex(newIndex);
        setCommand(history[newIndex]);
      }
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (historyIndex !== -1) {
        const newIndex = Math.min(history.length - 1, historyIndex + 1);
        if (newIndex === history.length - 1 && historyIndex === history.length - 1) {
          setHistoryIndex(-1);
          setCommand('');
        } else {
          setHistoryIndex(newIndex);
          setCommand(history[newIndex]);
        }
      }
    }
  };

  return (
    <div className={styles.container}>
      <div ref={messageLogRef} className={styles.debugMessages}>
        {debugMessages}
      </div>
      <div className={styles.evalSection}>
        <div className={styles.inputGroup}>
          <input
            type="text"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Enter command to evaluate..."
            className={styles.commandInput}
          />
          <button onClick={handleEvaluate} className={styles.evalButton}>
            Evaluate
          </button>
        </div>
      </div>
    </div>
  )
}
