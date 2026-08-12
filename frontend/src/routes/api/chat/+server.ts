import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

// Real OpenAI-compatible chat endpoint that proxies to llama-server
export const POST: RequestHandler = async ({ request }) => {
  try {
    const body = await request.json();
    
    // OpenAI-compatible request format
    const openaiRequest = {
      model: body.model || 'tinyllama',
      messages: body.messages,
      temperature: body.temperature || 0.7,
      max_tokens: body.max_tokens || 2048,
      stream: body.stream || false
    };

    // Use master token from DecentraAI backend
    const API_TOKEN = '8f8858d04611a013787ffe98e86c5053aaae42b931d4acbffd08d070f95ede73';

    // Proxy to llama-server (assumes it's running on localhost:8080)
    const response = await fetch('http://127.0.0.1:8080/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${API_TOKEN}`
      },
      body: JSON.stringify(openaiRequest)
    });

    if (!response.ok) {
      const errorText = await response.text();
      throw new Error(`llama-server error: ${response.status} - ${errorText}`);
    }

    if (body.stream) {
      // Return streaming response
      return new Response(response.body, {
        headers: {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          'Connection': 'keep-alive'
        }
      });
    } else {
      // Return non-streaming response
      const data = await response.json();
      return json(data);
    }
  } catch (error) {
    console.error('Chat API error:', error);
    return json(
      { error: error instanceof Error ? error.message : 'Failed to process chat request' },
      { status: 500 }
    );
  }
};