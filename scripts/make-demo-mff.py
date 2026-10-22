"""Create a synthetic continuous EEG recording for viewer smoke tests. Never overwrites."""
from pathlib import Path
import argparse
import math
import struct

parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
args = parser.parse_args()
p = args.output
p.mkdir(parents=True, exist_ok=False)
channels, rate, seconds = 32, 1000, 60
(p / 'info.xml').write_text('<fileInfo><recordTime>2026-01-01T00:00:00.000000Z</recordTime></fileInfo>')
(p / 'info1.xml').write_text('<dataInfo><fileDataType><EEG/></fileDataType></dataInfo>')
sensors = ''.join(f'<sensor><number>{c+1}</number><type>0</type><name>E{c+1}</name><unit>uV</unit></sensor>' for c in range(channels))
(p / 'sensorLayout.xml').write_text(f'<sensorLayout><sensors>{sensors}</sensors></sensorLayout>')
with (p / 'signal1.bin').open('wb') as f:
    for second in range(seconds):
        if second == 0:
            f.write(struct.pack('<4I', 1, 16+8*channels, channels*rate*4, channels))
            f.write(struct.pack(f'<{channels}I', *(c*rate*4 for c in range(channels))))
            f.write(struct.pack(f'<{channels}I', *([(rate<<8)|32]*channels)))
        else:
            f.write(struct.pack('<I', 0))
        for channel in range(channels):
            row = []
            for i in range(rate):
                t = second + i/rate
                value = 20*math.sin(2*math.pi*(7+channel/8)*t) + 6*math.sin(2*math.pi*1.2*t) + channel*2
                if second % 10 == 5 and i == 0:
                    value += 140
                if channel == 0 and second == 12 and 200 <= i < 250:
                    value = float('nan')
                row.append(value)
            f.write(struct.pack(f'<{rate}f', *row))
(p / 'epochs.xml').write_text(f'<epochs><epoch><beginTime>0</beginTime><endTime>{seconds*1000000}</endTime><firstBlock>1</firstBlock><lastBlock>{seconds}</lastBlock></epoch></epochs>')
events = ''.join(f'<event><code>STIM</code><label>Synthetic spike {s}</label><beginTime>2026-01-01T00:00:{s:02}.000000Z</beginTime><duration>1000</duration></event>' for s in range(5,seconds,10))
(p / 'Events_demo.xml').write_text(f'<eventTrack>{events}</eventTrack>')
print(p.resolve())
