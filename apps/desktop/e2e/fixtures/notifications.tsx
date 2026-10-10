import { createRoot } from 'react-dom/client';
import { toast } from 'sonner';
import { NotificationSurface } from '../../src/components/ui/Notifications';
import { applyTheme, isLightTheme, type ThemeId } from '../../src/lib/theme';
import '../../src/index.css';
const theme = (new URLSearchParams(location.search).get('theme') ?? 'dark') as ThemeId;
applyTheme(theme);
createRoot(document.getElementById('root')!).render(<div style={{ height: '100vh', padding: 24 }}>
  <button onClick={() => toast.warning('Needs attention: review your provider connection.', { description: 'This message stays readable above the workspace.', duration: 60000 })}>Show warning</button>
  <button onClick={() => { for (let index = 0; index < 6; index++) toast.error(`Attention ${index}: ${'Long diagnostic details. '.repeat(40)}`, { duration: 60000 }); }}>Show many</button>
  <textarea aria-label="Composer" style={{ position:'fixed',bottom:16,left:'25%',height:120,width:'50%' }} />
  <NotificationSurface light={isLightTheme(theme)} closeLabel="Close" />
</div>);
