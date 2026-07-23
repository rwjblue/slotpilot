# Receive spectrum and waterfall

Phase 2 derives a bounded visualization model from SlotPilot-owned canonical
12 kHz mono samples. It is pure worker-side DSP: it opens no device, runs no
callback, invokes no decoder, persists no bulk rows, renders no pixels, and
creates no output, rig, PTT, transmit, scheduler, or RF behavior.

## Signal model

The default policy uses a 1,024-sample periodic Hann window with 512 samples
of overlap. Rows therefore begin every 512 canonical samples (42.666 ms).
The retained passband is 0 through 5,000 Hz, inclusive by FFT-bin center, and
each row carries:

- daemon process and input stream generations;
- the exact FT8 slot and canonical sample offset;
- exact UTC start time in integer microseconds;
- the Hann window identity;
- ordered bins with integer FFT index, center frequency in millihertz, and
  peak magnitude in integer millidecibels relative to signed-16-bit full
  scale;
- optional reset evidence on the first row after an invalidation.

The adapter corrects for Hann coherent gain and doubles non-DC/non-Nyquist
positive-frequency magnitudes. Silence and values below the useful numerical
floor are represented as -120,000 mdBFS. FFT floating-point results are
quantized to integer mdBFS; generated-tone tests allow only the documented
small rounding tolerance.

## Bounds and backpressure

Configuration accepts only power-of-two FFT lengths from 256 through 4,096,
overlap no greater than 75 percent, a passband within the 6 kHz canonical
Nyquist frequency, 1 through 120 retained rows, and publication cadence from
50 through 2,000 ms. It rejects configurations exceeding:

- 2 MiB of conservative model allocation;
- 4,096 samples in one worker-side input chunk;
- 64 FFT rows of work in one push;
- 2,049 positive-frequency bins in one row.

The default model retains 60 rows and marks publication due at most every
250 ms. There is one pending publication token. If the consumer is absent or
slow, later due tokens replace it and increment an observable coalescing
counter. Oldest rows are evicted at fixed capacity with a separate observable
counter. Snapshot copying occurs only when a consumer takes the token;
capture, resampling, slot assembly, and decode never wait for a client.

## Continuity and reset

Chunks must carry contiguous canonical offsets within an exact typed FT8 slot.
A normal next-slot boundary clears overlap so an FFT never spans two slots.
Process-generation change, stream/device-generation change, non-contiguous
timeline position, an explicit timeline invalidation, or clock-health loss
clears overlap, retained rows, and pending publication state. The first later
row carries the exact reset reason so recovery cannot hide the discontinuity.

The model can copy bounded chunks from a canonical `Ft8ReceiveWindow`; a later
daemon composition issue owns when those chunks are submitted. This slice
adds no public wire schema or durable schema.
