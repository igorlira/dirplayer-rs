import styles from './styles.module.css';
import { ThemePreference, useTheme } from '../../utils/theme';

const OPTIONS: { value: ThemePreference; label: string; icon: JSX.Element }[] = [
  {
    value: 'system',
    label: 'Match system appearance',
    icon: (
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <line x1="8" y1="21" x2="16" y2="21" />
        <line x1="12" y1="17" x2="12" y2="21" />
      </svg>
    ),
  },
  {
    value: 'light',
    label: 'Light',
    icon: (
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="4.5" />
        <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
      </svg>
    ),
  },
  {
    value: 'dark',
    label: 'Dark',
    icon: (
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
      </svg>
    ),
  },
];

export default function ThemeToggle() {
  const { preference, setPreference } = useTheme();

  return (
    <div className={styles.themeToggle} role="group" aria-label="Colour theme">
      {OPTIONS.map(option => (
        <button
          key={option.value}
          className={`${styles.themeOption} ${preference === option.value ? styles.themeOptionActive : ''}`}
          onClick={() => setPreference(option.value)}
          title={option.label}
          aria-label={option.label}
          aria-pressed={preference === option.value}
        >
          {option.icon}
        </button>
      ))}
    </div>
  );
}
