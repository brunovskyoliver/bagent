import Accelerate

/// Real-FFT spectrum analyzer for 16 kHz mono PCM audio.
///
/// Takes raw PCM samples from WhisperKit's `AudioProcessor.audioSamples` and maps
/// them into `bandCount` perceptually log-spaced frequency bands (80 Hz – 8 kHz).
///
/// Not thread-safe; always call `analyze(_:)` from the same actor/queue (main actor).
final class SpectrumAnalyzer {

    static let fftN      = 1024         // FFT window size (64 ms at 16 kHz)
    static let bandCount = 24           // output frequency bands
    private static let sampleRate: Float = 16_000

    private let halfN: Int
    private let log2n: vDSP_Length
    private let fftSetup: OpaquePointer  // vDSP_FFTSetup
    private var window:   [Float]        // Hann window
    private var realBuf:  [Float]        // split-complex real part
    private var imagBuf:  [Float]        // split-complex imag part
    private let bandEdges: [Int]         // bin-index edges, length = bandCount + 1

    init() {
        let n    = Self.fftN
        halfN    = n / 2
        log2n    = vDSP_Length(log2f(Float(n)))
        fftSetup = vDSP_create_fftsetup(log2n, FFTRadix(kFFTRadix2))!
        window   = [Float](repeating: 0, count: n)
        vDSP_hann_window(&window, vDSP_Length(n), Int32(vDSP_HANN_NORM))
        realBuf  = [Float](repeating: 0, count: n / 2)
        imagBuf  = [Float](repeating: 0, count: n / 2)

        // Log-spaced bin edges: 80 Hz → 8 000 Hz
        let hz    = Self.sampleRate / Float(n)
        let loB   = max(1, Int(80   / hz))        // ≈ bin 5
        let hiB   = min(n / 2 - 1, Int(8000 / hz)) // ≈ bin 511
        let logLo = log10(Float(loB))
        let logHi = log10(Float(hiB))
        bandEdges = (0...Self.bandCount).map { i in
            let t = Float(i) / Float(Self.bandCount)
            return Int(pow(10, logLo + t * (logHi - logLo)))
        }
    }

    deinit { vDSP_destroy_fftsetup(fftSetup) }

    /// Analyze the most recent samples and return `bandCount` values in 0 … 1.
    ///
    /// Reads the last `fftN` samples from the ring buffer and computes a
    /// log-compressed, globally-normalized magnitude spectrum.  The caller is
    /// responsible for smoothing the output over time.
    func analyze(_ samples: ContiguousArray<Float>) -> [Float] {
        let n    = Self.fftN
        let half = halfN
        guard samples.count >= 8 else { return [Float](repeating: 0, count: Self.bandCount) }

        // Copy last N samples into a working buffer (zero-pad front if needed)
        var buf = [Float](repeating: 0, count: n)
        let avail  = min(samples.count, n)
        let srcIdx = samples.count - avail
        for i in 0..<avail { buf[n - avail + i] = samples[srcIdx + i] }

        // Apply Hann window
        vDSP_vmul(buf, 1, window, 1, &buf, 1, vDSP_Length(n))

        // Pack interleaved floats → split complex, then real FFT in-place
        var mags = [Float](repeating: 0, count: half)
        realBuf.withUnsafeMutableBufferPointer { rp in
            imagBuf.withUnsafeMutableBufferPointer { ip in
                var split = DSPSplitComplex(realp: rp.baseAddress!, imagp: ip.baseAddress!)
                buf.withUnsafeBufferPointer { bp in
                    bp.baseAddress!.withMemoryRebound(to: DSPComplex.self, capacity: half) { cp in
                        vDSP_ctoz(cp, 2, &split, 1, vDSP_Length(half))
                    }
                }
                vDSP_fft_zrip(fftSetup, &split, 1, log2n, FFTDirection(FFT_FORWARD))
                vDSP_zvmags(&split, 1, &mags, 1, vDSP_Length(half))
            }
        }

        // log1p compression for perceptual (loudness) scaling
        var compressed = [Float](repeating: 0, count: half)
        for i in 0..<half { compressed[i] = log1pf(mags[i]) }

        // Normalize by global peak
        var peak: Float = 0
        vDSP_maxv(compressed, 1, &peak, vDSP_Length(half))
        if peak > 1e-10 {
            var inv = 1.0 / peak
            vDSP_vsmul(compressed, 1, &inv, &compressed, 1, vDSP_Length(half))
        }

        // Reduce to log-spaced bands using the max bin within each band
        return (0..<Self.bandCount).map { i in
            let lo = min(bandEdges[i],     half - 1)
            let hi = min(bandEdges[i + 1], half)
            guard lo < hi else { return Float(0) }
            var v: Float = 0
            vDSP_maxv(Array(compressed[lo..<hi]), 1, &v, vDSP_Length(hi - lo))
            return v
        }
    }
}
