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

    // Use hardcoded token for now (in production, would come from user localStorage)
    const API_TOKEN = '32508c50e6bd1a7c149ac8a580c42cdf1afea10657a9c6961466b1897e240ba2';

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