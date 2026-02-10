import { Modal } from './Modal';
import { Button } from './Button';
import { AlertTriangle } from 'lucide-react';

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  message: string;
  confirmText?: string;
  variant?: 'danger' | 'warning';
  loading?: boolean;
}

export function ConfirmDialog({
  open, onClose, onConfirm, title, message,
  confirmText = '确认', variant = 'danger', loading
}: ConfirmDialogProps) {
  return (
    <Modal open={open} onClose={onClose} title={title} footer={
      <>
        <Button variant="ghost" size="sm" onClick={onClose}>取消</Button>
        <Button variant="danger" size="sm" onClick={onConfirm} loading={loading}>
          {confirmText}
        </Button>
      </>
    }>
      <div className="flex gap-3">
        <div className={`shrink-0 p-2 rounded-lg ${variant === 'danger' ? 'bg-danger/10 text-danger' : 'bg-warning/10 text-warning'}`}>
          <AlertTriangle size={20} />
        </div>
        <p className="text-sm text-text-secondary leading-relaxed">{message}</p>
      </div>
    </Modal>
  );
}
