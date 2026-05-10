import logoUrl from '../../assets/logo128.png';

type ErrorOverlayProps = {
  message: string;
  compact?: boolean;
};

export default function ErrorOverlay({ message, compact = false }: ErrorOverlayProps) {
  const logoSize = compact ? 18 : 26;
  const brandFontSize = compact ? '11px' : '13px';
  const iconSize = compact ? 28 : 52;
  const titleFontSize = compact ? '13px' : '20px';
  const messageFontSize = compact ? '11px' : '14px';
  const brandTop = compact ? '12px' : '28px';
  const brandGap = compact ? '6px' : '9px';

  return (
    <div style={{
      width: '100%',
      height: '100%',
      position: 'relative',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: '#1a1a1a',
      fontFamily: 'sans-serif',
    }}>
      <div style={{
        position: 'absolute',
        top: brandTop,
        display: 'flex',
        alignItems: 'center',
        gap: brandGap,
      }}>
        <img src={logoUrl} alt="" width={logoSize} height={logoSize} style={{ borderRadius: compact ? '3px' : '5px', opacity: 0.5 }} />
        <span style={{ fontSize: brandFontSize, fontWeight: 600, color: '#505050', letterSpacing: '0.01em' }}>DirPlayer</span>
      </div>
      <div style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: compact ? '8px' : '16px',
        padding: compact ? '16px' : '32px',
        boxSizing: 'border-box',
        maxWidth: compact ? '100%' : '600px',
        textAlign: 'center',
      }}>
        <svg width={iconSize} height={iconSize} viewBox="0 0 24 24" fill="none" stroke="#f5a623" strokeWidth={compact ? 2 : 1.5} strokeLinecap="round" strokeLinejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
          <line x1="12" y1="9" x2="12" y2="13"/>
          <line x1="12" y1="17" x2="12.01" y2="17"/>
        </svg>
        <div style={{ fontSize: titleFontSize, fontWeight: 'bold', color: '#fff' }}>Movie failed to load</div>
        <div style={{ fontSize: messageFontSize, color: '#888', wordBreak: 'break-word' }}>{message}</div>
      </div>
    </div>
  );
}
