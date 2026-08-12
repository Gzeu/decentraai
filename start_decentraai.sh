#!/bin/bash

# Script to start the DecentraAI node.
# Launches the P2P swarm and the inference server.

DATA_DIR=~/.decentraai
CONFIG_FILE=$DATA_DIR/config.yaml

echo "Starting DecentraAI node..."
echo "Using config: $CONFIG_FILE"

# Start the P2P swarm in the background, logging output
echo "Starting P2P swarm..."
decentraai swarm start --config "$CONFIG_FILE" > "$DATA_DIR/swarm.log" 2>&1 &
SWARM_PID=$!
echo "Swarm started with PID $SWARM_PID"

# Brief pause to let the swarm initialize
sleep 3

# Start the inference server with TinyLlama, pointing to the llama-server binary
echo "Starting inference server with TinyLlama..."
decentraai serve start --config "$CONFIG_FILE" --model "$DATA_DIR/models/tinyllama.gguf" --binary /home/i7/llama.cpp/build/bin/llama-server > "$DATA_DIR/serve.log" 2>&1 &
SERVE_PID=$!
echo "Server started with PID $SERVE_PID"

echo "Both processes launched. Tail the logs to monitor:"
echo "  - Swarm log: $DATA_DIR/swarm.log"
echo "  - Serve log: $DATA_DIR/serve.log"

# Wait for the serve process; exit if it crashes
wait $SERVE_PID
EXIT_CODE=$?
echo "Inference server exited with code $EXIT_CODE."