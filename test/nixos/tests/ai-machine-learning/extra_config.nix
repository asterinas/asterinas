{ config, lib, pkgs, ... }:
let
  test_torch = pkgs.writeTextFile {
    name = "test_torch.py";
    text = ''
      #!/usr/bin/env python3.12

      import unittest
      import math
      import torch


      class TestTorchEnv(unittest.TestCase):
        def test_import_and_version(self):
          # Torch should expose a version string and it should be >= 1.x
          self.assertIsNotNone(torch.__version__)
          major = int(torch.__version__.split(".")[0])
          self.assertGreaterEqual(major, 1)

        def test_cuda_available_flag_type(self):
          # torch.cuda.is_available() should always return a bool
          available = torch.cuda.is_available()
          self.assertIsInstance(available, bool)


      class TestAutograd(unittest.TestCase):
        def test_basic_autograd_linear(self):
          # Simple linear model: y = x @ w + b, then compute MSE-like loss
          x = torch.randn(4, 3, requires_grad=True)
          w = torch.randn(3, 2, requires_grad=True)
          b = torch.randn(2, requires_grad=True)

          # Forward pass
          y = x @ w + b  # shape: (4, 2)
          loss = y.pow(2).mean()
          # Backward pass: compute gradients
          loss.backward()

          # Gradients must be populated
          self.assertIsNotNone(x.grad)
          self.assertIsNotNone(w.grad)
          self.assertIsNotNone(b.grad)

          # Gradient shapes must match their corresponding tensors
          self.assertEqual(x.grad.shape, x.shape)
          self.assertEqual(w.grad.shape, w.shape)
          self.assertEqual(b.grad.shape, b.shape)


      class TestNNAndOptim(unittest.TestCase):
        def test_linear_regression_training(self):
          # Synthetic 1D linear regression: y = 2x + 3 + noise
          torch.manual_seed(0)

          N = 200
          X = torch.randn(N, 1)
          true_w, true_b = 2.0, 3.0
          Y = true_w * X + true_b + 0.1 * torch.randn(N, 1)

          # Single linear layer is enough for 1D linear regression
          model = torch.nn.Linear(1, 1)
          optimizer = torch.optim.SGD(model.parameters(), lr=0.1)
          loss_fn = torch.nn.MSELoss()

          prev_loss = None
          for _ in range(50):
            optimizer.zero_grad()
            pred = model(X)
            loss = loss_fn(pred, Y)
            loss.backward()
            optimizer.step()

            # Loss should generally decrease, allow a small tolerance
            if prev_loss is not None:
              self.assertLessEqual(loss.item(), prev_loss + 1e-3)
            prev_loss = loss.item()

          # Extract learned parameters
          est_w = model.weight.item()
          est_b = model.bias.item()

          # Final loss should be finite and parameters close to ground truth
          self.assertTrue(math.isfinite(prev_loss))
          self.assertTrue(math.isclose(est_w, true_w, rel_tol=0.2))
          self.assertTrue(math.isclose(est_b, true_b, rel_tol=0.2))


      @unittest.skipUnless(torch.cuda.is_available(), "CUDA is not available")
      class TestCuda(unittest.TestCase):
        def test_cuda_tensor_basic(self):
          # Basic GPU matmul test: ensure tensors live and compute on CUDA
          device = torch.device("cuda")
          x = torch.randn(100, 100, device=device)
          y = torch.randn(100, 100, device=device)
          z = x @ y

          # Result must reside on CUDA and have the expected shape
          self.assertEqual(z.device.type, "cuda")
          self.assertEqual(z.shape, (100, 100))


      if __name__ == "__main__":
        unittest.main(verbosity=2)
    '';
  };
  test_tensorflow = pkgs.writeTextFile {
    name = "test_tensorflow.py";
    text = ''
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
    '';
  };
in {
  environment.systemPackages = with pkgs; [
    (python3.withPackages (p: with p; [ torch tensorflow pytest ]))
    ollama
  ];

  environment.loginShellInit = ''
    [ ! -e /tmp/test_torch.py ] && ln -s ${test_torch} /tmp/test_torch.py
    [ ! -e /tmp/test_tensorflow.py ] && ln -s ${test_tensorflow} /tmp/test_tensorflow.py
  '';
}
