import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import { faPlay, faStop, faRotateBack } from '@fortawesome/free-solid-svg-icons'
import IconButton from '../IconButton'
import styles from './styles.module.css'
import { play, stop, reset } from 'vm-rust'

export default function PlaybackControls() {
  return <div className={styles.container}>
    <IconButton icon={faPlay} onClick={() => { play() }} />
    <IconButton icon={faStop} onClick={() => { stop() }} />
    <IconButton icon={faRotateBack} onClick={() => { reset() }} />
  </div>
}
