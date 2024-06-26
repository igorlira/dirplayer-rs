import { useEffect, useRef } from "react";

export function useMountEffect(effect: () => void) {
  const isMounted = useRef(false);
  useEffect(() => {
    if (!isMounted.current) {
      effect();
      isMounted.current = true;
    }
  }, []);
}
