"""Signal-level regression checks, independent of any room recording."""

import unittest

import numpy as np

from denoise_offline import FFT_SIZE, RATE, denoise, noise_profile, rms


class DenoiseTests(unittest.TestCase):
    def test_bypass_reconstructs_edges_and_non_frame_lengths(self):
        rng = np.random.default_rng(124)
        for count in (1, 127, 128, 255, 256, 257, 16003):
            samples = rng.uniform(-0.9, 0.9, count)
            samples[0], samples[-1] = 0.8, -0.7
            actual = denoise(samples, np.ones(FFT_SIZE // 2 + 1), 0)
            np.testing.assert_allclose(actual, samples, atol=1e-12, rtol=0)

    def test_silence_stays_finite_and_silent(self):
        output = denoise(np.zeros(16003), np.zeros(FFT_SIZE // 2 + 1))
        self.assertTrue(np.all(np.isfinite(output)))
        self.assertEqual(float(np.max(np.abs(output))), 0)

    def test_loud_tone_is_not_treated_as_noise(self):
        t = np.arange(RATE) / RATE
        samples = 0.5 * np.sin(2 * np.pi * 997 * t)
        actual = denoise(samples, np.full(FFT_SIZE // 2 + 1, 1e-8))
        self.assertEqual(len(actual), len(samples))
        self.assertLess(rms(actual[1024:] - samples[1024:]), 0.001)

    def test_independent_noise_and_known_signal_improve_reconstruction_snr(self):
        rng = np.random.default_rng(247)
        calibration = rng.normal(0, 0.04, RATE * 3)
        noise = rng.normal(0, 0.04, RATE * 4)
        t = np.arange(len(noise)) / RATE
        # Known, modulated multi-harmonic signal; this is not real speech.
        envelope = 0.5 - 0.5 * np.cos(2 * np.pi * 3 * t)
        clean = envelope * sum(0.05 / harmonic * np.sin(2 * np.pi * 180 * harmonic * t)
                               for harmonic in range(1, 15))
        mixed = clean + noise
        profile = noise_profile(calibration, (0, 3))
        actual = denoise(mixed, profile)
        self.assertLess(rms(actual[1024:] - clean[1024:]), 0.8 * rms(noise[1024:]))
        residual = denoise(noise, profile)
        self.assertLess(rms(residual[1024:]), 0.4 * rms(noise[1024:]))

    def test_invalid_inputs_and_calibration_are_rejected(self):
        samples = np.zeros(RATE)
        for interval in ((0, 0), (-1, 0.5), (0, 2), (0, 0.02)):
            with self.assertRaises(ValueError):
                noise_profile(samples, interval)
        with self.assertRaises(ValueError):
            denoise(samples, np.ones(128))
        with self.assertRaises(ValueError):
            denoise(samples, np.ones(129), float("nan"))


if __name__ == "__main__":
    unittest.main()
