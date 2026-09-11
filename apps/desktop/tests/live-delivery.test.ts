import { LiveAudioQueue } from '../src/features/live/liveAudioQueue';
import { TerminalPcmDelivery } from '../src/features/voice/terminalPcmDelivery';
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
  const pipelined: number[] = [];
  const acknowledgements: Array<() => void> = [];
  const network = new LiveAudioQueue(2, chunk => {
    pipelined.push(chunk[0]);
    return new Promise<void>(resolve => acknowledgements.push(resolve));
  }, () => { throw new Error('healthy remote acknowledgements must not overflow'); }, 4);
  for (let i = 0; i < 6; i++) network.append(new Uint8Array([i, i]));
  check(pipelined.join(',') === '0,1,2,3', 'network window sends ordered PCM without waiting for one RTT per packet');
  acknowledgements[2](); await new Promise(resolve => setTimeout(resolve, 0));
  check(pipelined.join(',') === '0,1,2,3,4', 'an acknowledgement releases exactly one bounded slot');
  network.pause(); acknowledgements[0](); await new Promise(resolve => setTimeout(resolve, 0));
  network.resume(); network.append(new Uint8Array([7, 7]));
  check(pipelined.join(',') === '0,1,2,3,4,7', 'route changes discard queued stale PCM without reordering new audio');
  network.close(); acknowledgements.forEach(ack => ack());
  let reconnects = 0;
  const recoveryPackets: number[] = [];
  let releaseStall!: () => void;
  const stalledNetwork = new LiveAudioQueue(2, async bytes => { recoveryPackets.push(bytes[0]); await new Promise<void>(resolve => { releaseStall = resolve; }); },
    () => { throw new Error('network congestion must reconnect instead of terminating Live'); }, 1, () => { reconnects++; });
  for (let i = 0; i < 10; i++) check(stalledNetwork.append(new Uint8Array([i,i])), 'overloaded remote capture remains resumable');
  check(reconnects === 1, 'a full remote queue enters the reconnect protocol exactly once');
  releaseStall(); await new Promise(resolve => setTimeout(resolve, 0));
  stalledNetwork.resume(); stalledNetwork.append(new Uint8Array([42,42]));
  check(recoveryPackets.join(',') === '0,42', 'reconnection discards old audio and resumes with fresh PCM');
  releaseStall(); stalledNetwork.close();
  const tail: number[][] = [];
  const finalPacket = new LiveAudioQueue(4, async bytes => { tail.push([...bytes]); }, () => { throw new Error('tail delivery failed'); });
  finalPacket.append(new Uint8Array([1, 2]));
  await finalPacket.finish();
  check(tail.length === 1 && tail[0].join(',') === '1,2', 'dictation finalization delivers short terminal PCM before asking the provider to finish');
  check(!finalPacket.append(new Uint8Array([3,4])), 'finished audio cannot accept later capture samples');
  const resumedPackets: number[] = [];
  const resumable = new LiveAudioQueue(2, async chunk => { resumedPackets.push(chunk[0]); }, () => { throw new Error('pause must not become a terminal capture failure'); });
  const recorder = new TerminalPcmDelivery(chunk => resumable.append(chunk));
  recorder.deliver(new Uint8Array([1, 1])); resumable.pause();
  check(recorder.deliver(new Uint8Array([2, 2])) === 'accepted', 'in-flight worklet samples during pause do not terminate the recorder');
  resumable.resume(); recorder.deliver(new Uint8Array([3, 3])); await Promise.resolve(); await Promise.resolve();
  check(resumedPackets.join(',') === '1,3', 'resume discards paused samples and continues ordered capture');
  const snapshot: LiveSnapshot = { id:'s',mode:'incremental',model:'qwen-vl',phase:'listening',sampleRate:16000,startedAt:'',sequence:3,entries:[],error:null,metrics:{framesReceived:0,framesSubmitted:0,framesReplaced:0,audioMs:0,lastResponseMs:null,omittedEntries:0} };
  check(applyLiveEvent(snapshot,{sessionId:'old',sequence:10,type:'state',phase:'error',error:'old'}) === snapshot,'late events from another session do not replace the current session');
  check(applyLiveEvent(snapshot,{sessionId:'s',sequence:2,type:'state',phase:'error',error:'old'}) === snapshot,'out-of-order events are ignored');
  let updated = snapshot;
  for(let i=0;i<170;i++) updated=applyLiveEvent(updated,{sessionId:'s',sequence:4+i,type:'entry',entry:{id:String(i),text:'event',role:'assistant',atMs:i,complete:false}});
  check(updated.entries.length === 160 && updated.entries[0].id === '10','long sessions retain a bounded reading view');
  console.log('ok - Live backpressure, cancellation, session identity, and bounded records');
}
void run().catch(error => { console.error(error); throw error; });
