#!/usr/bin/env python3.12

import unittest
import math

import tensorflow as tf


class TestTFEnv(unittest.TestCase):
    def test_import_and_version(self):
        # Ensure TensorFlow has a version string and major version >= 2
        self.assertIsNotNone(tf.__version__)
        major = int(tf.__version__.split(".")[0])
        self.assertGreaterEqual(major, 2)

    def test_gpu_available_flag_type(self):
        # Check that listing GPUs returns a list (may be empty)
        gpus = tf.config.list_physical_devices("GPU")
        self.assertIsInstance(gpus, list)


class TestAutograd(unittest.TestCase):
    def test_basic_autograd_linear(self):
        # Linear layer: y = x @ w + b, then compute gradients of MSE loss
        x = tf.random.normal((4, 3))
        w = tf.Variable(tf.random.normal((3, 2)))
        b = tf.Variable(tf.random.normal((2,)))

        with tf.GradientTape() as tape:
            y = tf.matmul(x, w) + b  # shape (4, 2)
            loss = tf.reduce_mean(tf.square(y))

        grads = tape.gradient(loss, [w, b])

        # We only assert gradients for w and b here
        self.assertIsNotNone(grads[0])  # grad w.r.t w
        self.assertIsNotNone(grads[1])  # grad w.r.t b

        self.assertEqual(grads[0].shape, w.shape)
        self.assertEqual(grads[1].shape, b.shape)


class TestNNAndOptim(unittest.TestCase):
    def test_linear_regression_training(self):
        # Simple 1D linear regression: learn y = 2x + 3 with noise
        tf.random.set_seed(0)

        N = 200
        X = tf.random.normal((N, 1))
        true_w, true_b = 2.0, 3.0
        Y = true_w * X + true_b + 0.1 * tf.random.normal((N, 1))

        # Scalar parameters for y = w * x + b
        w = tf.Variable(tf.random.normal(()))
        b = tf.Variable(tf.random.normal(()))

        learning_rate = 0.1
        prev_loss = None

        for _ in range(100):
            with tf.GradientTape() as tape:
                # Predicted values: (N, 1)
                y_pred = w * X + b
                # Mean squared error
                loss = tf.reduce_mean(tf.square(y_pred - Y))

            dw, db = tape.gradient(loss, [w, b])

            # Manual SGD update
            w.assign_sub(learning_rate * dw)
            b.assign_sub(learning_rate * db)

            loss_val = float(loss.numpy())
            if prev_loss is not None:
                # Allow small numerical noise while requiring general decrease
                self.assertLessEqual(loss_val, prev_loss + 1e-3)
            prev_loss = loss_val

        est_w = float(w.numpy())
        est_b = float(b.numpy())

        self.assertTrue(math.isfinite(prev_loss))
        self.assertTrue(math.isclose(est_w, true_w, rel_tol=0.2))
        self.assertTrue(math.isclose(est_b, true_b, rel_tol=0.2))


@unittest.skipUnless(tf.config.list_physical_devices("GPU"), "GPU is not available")
class TestGpu(unittest.TestCase):
    def test_gpu_tensor_basic(self):
        # Perform a matrix multiplication on the first GPU
        with tf.device("/GPU:0"):
            x = tf.random.normal((100, 100))
            y = tf.random.normal((100, 100))
            z = tf.matmul(x, y)

        # In eager mode, tensor.device is a string containing device info
        self.assertIn("GPU", z.device)
        self.assertEqual(z.shape, (100, 100))


if __name__ == "__main__":
    unittest.main(verbosity=2)
