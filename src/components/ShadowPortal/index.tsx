import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

type ShadowPortalProps = {
  children: React.ReactNode;
  style?: React.CSSProperties;
};

export default function ShadowPortal({ children, style }: ShadowPortalProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [shadowRoot, setShadowRoot] = useState<ShadowRoot | null>(null);

  useEffect(() => {
    if (hostRef.current && !hostRef.current.shadowRoot) {
      setShadowRoot(hostRef.current.attachShadow({ mode: 'open' }));
    }
  }, []);

  return (
    <div ref={hostRef} style={style}>
      {shadowRoot && createPortal(children, shadowRoot as unknown as Element)}
    </div>
  );
}
