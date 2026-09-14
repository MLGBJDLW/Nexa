export interface ScrollMetrics {
  nearBottom: boolean;
  overflow: boolean;
}

/** Follow rendered geometry, including delayed text paints and tool card resize.
 * Only navigation away from the bottom releases the latch; growing content does not. */
export function observeScrollFollow(
  container: HTMLElement,
  content: HTMLElement,
  following: { current: boolean },
  onMetrics: (metrics: ScrollMetrics) => void,
  threshold = 40,
) {
  let frame: number | null = null;
  let previousTop = container.scrollTop;
  let previousHeight = container.scrollHeight;
  let previousViewport = container.clientHeight;
  let touchY: number | null = null;
  const measure = () => {
    onMetrics({
      nearBottom: container.scrollHeight - container.scrollTop - container.clientHeight <= threshold,
      overflow: container.scrollHeight > container.clientHeight + 8,
    });
  };
  const remember = () => {
    previousTop = container.scrollTop;
    previousHeight = container.scrollHeight;
    previousViewport = container.clientHeight;
  };
  const reconcile = () => {
    const layoutChanged = previousHeight !== container.scrollHeight || previousViewport !== container.clientHeight;
    if (!layoutChanged && container.scrollTop < previousTop - 1) {
      following.current = false;
    } else if (container.scrollTop > previousTop + 1 && (
      container.scrollHeight - container.scrollTop - container.clientHeight <= threshold
      || previousHeight - container.scrollTop - previousViewport <= threshold
    )) {
      // The user may reach the old bottom just before the next layout is painted.
      following.current = true;
    }
  };
  const schedule = () => {
    if (frame != null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      reconcile();
      if (following.current) container.scrollTop = container.scrollHeight;
      remember();
      measure();
    });
  };
  const scroll = () => {
    reconcile();
    remember();
    measure();
  };
  // Nested trace/code panels own their input; do not pause the outer timeline.
  const ownsInput = (target: EventTarget | null) => {
    for (let node = target instanceof Element ? target : null; node && node !== container; node = node.parentElement) {
      if (node.scrollHeight > node.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(node).overflowY)) return false;
    }
    return true;
  };
  const wheel = (event: WheelEvent) => {
    if (!event.ctrlKey && !event.shiftKey && event.deltaY < 0 && ownsInput(event.target)) following.current = false;
  };
  const touchStart = (event: TouchEvent) => { touchY = event.touches[0]?.clientY ?? null; };
  const touchMove = (event: TouchEvent) => {
    const next = event.touches[0]?.clientY ?? null;
    if (next != null && touchY != null && next > touchY && ownsInput(event.target)) following.current = false;
    touchY = next;
  };
  const keydown = (event: KeyboardEvent) => {
    if ((event.target as Element)?.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (ownsInput(event.target) && (['ArrowUp', 'PageUp', 'Home'].includes(event.key) || (event.key === ' ' && event.shiftKey))) following.current = false;
  };
  container.addEventListener('scroll', scroll, { passive: true });
  container.addEventListener('wheel', wheel, { passive: true });
  container.addEventListener('touchstart', touchStart, { passive: true });
  container.addEventListener('touchmove', touchMove, { passive: true });
  container.addEventListener('keydown', keydown);
  const observer = new ResizeObserver(schedule);
  observer.observe(content);
  observer.observe(container);
  schedule();
  return () => {
    observer.disconnect();
    if (frame != null) cancelAnimationFrame(frame);
    container.removeEventListener('scroll', scroll);
    container.removeEventListener('wheel', wheel);
    container.removeEventListener('touchstart', touchStart);
    container.removeEventListener('touchmove', touchMove);
    container.removeEventListener('keydown', keydown);
  };
}
