import { LiveAudioQueue } from '../src/features/live/liveAudioQueue';
import { applyLiveEvent, type LiveSnapshot } from '../src/features/live/liveTransport';

function check(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }
async function run() {
  const sent: Uint8Array[] = []; let unblock!: () => void; let failures = 0;
  const gate = new Promise<void>(resolve => { unblock = resolve; });
  const queue = new LiveAudioQueue(4, async chunk => { sent.push(chunk); await gate; }, () => { failures++; });
  check(queue.append(new Uint8Array([1, 2])), 'first worklet chunk accepted');
  check(queue.append(new Uint8Array([3, 4])), 'second chunk completes one packet');
  check(sent.length === 1 && sent[0].join(',') === '1,2,3,4', 'worklet data stays ordered and is grouped');
  for (let i = 0; i < 8; i++) check(queue.append(new Uint8Array([0,0,0,0])), 'bounded backlog accepts eight packets');
  check(!queue.append(new Uint8Array([0,0,0,0])), 'a stalled sink closes on overflow');
  unblock(); await Promise.resolve(); await Promise.resolve();
  check(sent.length === 1 && failures === 1, 'overflow discards backlog and reports once');
  check(!queue.append(new Uint8Array([0,0])), 'closed capture cannot resume after the sink recovers');
  const snapshot: LiveSnapshot = { id:'s',mode:'incremental',model:'qwen-vl',phase:'listening',sampleRate:16000,startedAt:'',sequence:3,entries:[],error:null,metrics:{framesReceived:0,framesSubmitted:0,framesReplaced:0,audioMs:0,lastResponseMs:null,omittedEntries:0} };
  check(applyLiveEvent(snapshot,{sessionId:'old',sequence:10,type:'state',phase:'error',error:'old'}) === snapshot,'late events from another session do not replace the current session');
  check(applyLiveEvent(snapshot,{sessionId:'s',sequence:2,type:'state',phase:'error',error:'old'}) === snapshot,'out-of-order events are ignored');
  let updated = snapshot;
  for(let i=0;i<170;i++) updated=applyLiveEvent(updated,{sessionId:'s',sequence:4+i,type:'entry',entry:{id:String(i),text:'event',role:'assistant',atMs:i,complete:false}});
  check(updated.entries.length === 160 && updated.entries[0].id === '10','long sessions retain a bounded reading view');
  console.log('ok - Live backpressure, cancellation, session identity, and bounded records');
}
void run().catch(error => { console.error(error); throw error; });
