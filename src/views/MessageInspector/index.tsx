import { useEffect, useRef, useState } from 'react';
import styles from './styles.module.css';
import { eval_command } from "vm-rust";
import { useAppSelector } from '../../store/hooks';
import { selectDebugMessages } from '../../store/vmSlice';

export default function MessageInspector() {
  const [command, setCommand] = useState('');
  const debugMessages = useAppSelector(({ vm }) => selectDebugMessages(vm)).join('\n');
  const messageLogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    messageLogRef.current?.scrollTo(0, messageLogRef.current.scrollHeight);
  }, [debugMessages]);

  const handleEvaluate = () => {
    try {
      eval_command(command);
      setCommand('');
    } catch (err) {
      console.error(err);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleEvaluate();
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
