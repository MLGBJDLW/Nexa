/** Keep keystrokes ordered when native writes run off the window thread. */
export function createTerminalInputWriter(write: (sessionId: string, data: string) => Promise<void>) {
  const pending = new Map<string, { tail: Promise<void>; chars: number }>();
  return (sessionId: string, data: string): Promise<void> => {
    const previous = pending.get(sessionId);
    const chars = (previous?.chars ?? 0) + data.length;
    if (chars > 256 * 1024) return Promise.reject(new Error('Terminal input is busy; wait before sending more text.'));
    // Do not send the rest of a partially failed command (especially Enter).
    const tail = (previous?.tail ?? Promise.resolve()).then(() => write(sessionId, data));
    const entry = { tail, chars };
    pending.set(sessionId, entry);
    const retire = () => {
      const current = pending.get(sessionId);
      if (current === entry) pending.delete(sessionId);
      else if (current) current.chars -= data.length;
    };
    void tail.then(retire, retire);
    return tail;
  };
}
