"use client";

import { useCallback, useEffect, useRef, useState } from "react";

// Minimal typings for the Web Speech API (not part of the standard DOM lib).
interface SpeechAlternative {
  transcript: string;
}
interface SpeechResult {
  isFinal: boolean;
  0: SpeechAlternative;
}
interface SpeechResultEvent {
  resultIndex: number;
  results: { length: number; [index: number]: SpeechResult };
}
interface SpeechRecognitionLike {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onresult: ((event: SpeechResultEvent) => void) | null;
  onerror: ((event: { error: string }) => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
  abort(): void;
}
type SpeechRecognitionCtor = new () => SpeechRecognitionLike;

function recognitionCtor(): SpeechRecognitionCtor | null {
  if (typeof window === "undefined") return null;
  const scope = window as unknown as { SpeechRecognition?: SpeechRecognitionCtor; webkitSpeechRecognition?: SpeechRecognitionCtor };
  return scope.SpeechRecognition ?? scope.webkitSpeechRecognition ?? null;
}

const ERRORS: Record<string, string> = {
  "not-allowed": "Accès au micro refusé : autorisez-le dans le navigateur ou utilisez le champ texte.",
  "service-not-allowed": "La dictée vocale est désactivée sur ce navigateur.",
  "no-speech": "Aucune parole détectée.",
  "audio-capture": "Aucun micro disponible.",
  network: "La dictée vocale nécessite une connexion réseau.",
};

export interface SpeechInput {
  supported: boolean;
  listening: boolean;
  /** Words recognised but not yet final. */
  interim: string;
  error: string | null;
  toggle: () => void;
  stop: () => void;
}

/** French dictation through the Web Speech API. `onFinal` receives each finalised phrase. */
export function useSpeechInput(onFinal: (text: string) => void): SpeechInput {
  const [supported] = useState(() => recognitionCtor() !== null);
  const [listening, setListening] = useState(false);
  const [interim, setInterim] = useState("");
  const [error, setError] = useState<string | null>(null);
  const recognition = useRef<SpeechRecognitionLike | null>(null);
  const handler = useRef(onFinal);

  useEffect(() => {
    handler.current = onFinal;
  }, [onFinal]);

  const stop = useCallback(() => {
    recognition.current?.stop();
  }, []);

  useEffect(() => () => recognition.current?.abort(), []);

  const toggle = useCallback(() => {
    if (recognition.current) {
      recognition.current.stop();
      return;
    }
    const Ctor = recognitionCtor();
    if (!Ctor) return;
    const instance = new Ctor();
    instance.lang = "fr-FR";
    instance.continuous = true;
    instance.interimResults = true;
    instance.onresult = (event) => {
      let pending = "";
      for (let index = event.resultIndex; index < event.results.length; index += 1) {
        const result = event.results[index];
        if (result.isFinal) handler.current(result[0].transcript.trim());
        else pending += result[0].transcript;
      }
      setInterim(pending);
    };
    instance.onerror = (event) => {
      if (event.error !== "aborted") setError(ERRORS[event.error] ?? `Dictée interrompue (${event.error}).`);
    };
    instance.onend = () => {
      recognition.current = null;
      setListening(false);
      setInterim("");
    };
    setError(null);
    try {
      instance.start();
      recognition.current = instance;
      setListening(true);
    } catch {
      setError("Impossible de démarrer la dictée vocale.");
    }
  }, []);

  return { supported, listening, interim, error, toggle, stop };
}
