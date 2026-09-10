import { useNavigate } from 'react-router';
import { LiveWorkspace } from '../features/live/LiveWorkspace';
import { desktopLiveTransport } from '../features/live/desktopLiveTransport';
export function LivePage() {
  const navigate = useNavigate();
  return <LiveWorkspace transport={desktopLiveTransport} onSendToChat={initialMessage => navigate('/chat', { state: { initialMessage } })} />;
}
