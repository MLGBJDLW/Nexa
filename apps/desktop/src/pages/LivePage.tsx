import { useLocation, useNavigate } from 'react-router';
import { LiveWorkspace } from '../features/live/LiveWorkspace';
import { desktopLiveTransport } from '../features/live/desktopLiveTransport';
export function LivePage() {
  const navigate = useNavigate();
  const location = useLocation();
  return <LiveWorkspace transport={desktopLiveTransport} background={location.pathname !== '/live'} onReturn={() => navigate('/live')} onSendToChat={initialMessage => navigate('/chat', { state: { initialMessage } })} />;
}
