# AI & Machine Learning

This category covers deep learning frameworks, LLM tools, and inference engines.

## Deep Learning Frameworks

### PyTorch

[PyTorch](https://pytorch.org/) is an open-source machine learning library developed by Facebook's AI Research lab. It provides tensor computation with GPU acceleration and deep neural networks built on a tape-based automatic differentiation system.

#### Installation

```nix
environment.systemPackages = with pkgs; [
  (python3.withPackages (p: with p; [ torch ]))
];
```

#### Verified Usage

```python
{{#include test_pytorch.py}}
```

### TensorFlow

[TensorFlow](https://www.tensorflow.org/) is an end-to-end open-source platform for machine learning developed by Google.

#### Installation

```nix
environment.systemPackages = with pkgs; [
  (python3.withPackages (p: with p; [ tensorflow ]))
];
```

#### Verified Usage

```python
{{#include test_tensorflow.py}}
```

## LLM Inference Engines

### Ollama

[Ollama](https://ollama.com/) is a lightweight, extensible framework for running large language models locally.

#### Installation

```nix
environment.systemPackages = [ pkgs.ollama ];
```

#### Verified Usage

```bash
# Start ollama server
ollama serve

# List downloaded models
ollama list
```
