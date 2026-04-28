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
