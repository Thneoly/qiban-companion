import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from 'react';

type Point = { x: number; y: number };
type Gesture = Point & { pointerId: number; origin: Point; target: HTMLElement; moved: boolean };

/** Keep clicks available until a deliberate move; native dragging owns the pointer after handoff. */
export function useSceneDrag(options: {
  enabled: boolean;
  offset: Point;
  move: (point: Point) => void;
  nativeDrag?: () => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const gesture = useRef<Gesture | null>(null);
  const suppressClick = useRef(false);
  const [dragging, setDragging] = useState(false);
  function end() {
    const current = gesture.current;
    gesture.current = null;
    if (current?.target.hasPointerCapture(current.pointerId)) current.target.releasePointerCapture(current.pointerId);
    setDragging(false);
  }
  useEffect(() => {
    window.addEventListener('blur', end);
    return () => { window.removeEventListener('blur', end); };
  }, []);
  useEffect(() => { if (!options.enabled) end(); }, [options.enabled]);

  return {
    dragging,
    onPointerDown(event: PointerEvent<HTMLDivElement>) {
      if (!options.enabled || event.button !== 0 || !event.isPrimary || gesture.current) return;
      suppressClick.current = false;
      const target = (event.target as Element).closest<HTMLElement>('[data-pet-drag]');
      if (!target) return;
      target.setPointerCapture(event.pointerId);
      gesture.current = { x: event.clientX, y: event.clientY, pointerId: event.pointerId,
        origin: options.offset, target, moved: false };
    },
    onPointerMove(event: PointerEvent<HTMLDivElement>) {
      const current = gesture.current;
      if (!current || event.pointerId !== current.pointerId) return;
      const dx = event.clientX - current.x, dy = event.clientY - current.y;
      if (!current.moved && Math.hypot(dx, dy) < 6) return;
      current.moved = true;
      suppressClick.current = true;
      event.preventDefault();
      setDragging(true);
      if (options.nativeDrag) {
        // Release web capture before Windows takes over its window-move loop.
        end();
        void options.nativeDrag().catch(options.onError);
      } else {
        options.move({
          x: Math.max(-Math.max(0, innerWidth - 344), Math.min(8, current.origin.x + dx)),
          y: Math.max(-Math.max(0, innerHeight - 464), Math.min(8, current.origin.y + dy)),
        });
      }
    },
    onPointerUp: end,
    onPointerCancel: end,
    onLostPointerCapture: end,
    onClickCapture(event: MouseEvent<HTMLDivElement>) {
      if (suppressClick.current && event.detail > 0) {
        suppressClick.current = false;
        event.preventDefault(); event.stopPropagation();
      }
    },
  };
}
