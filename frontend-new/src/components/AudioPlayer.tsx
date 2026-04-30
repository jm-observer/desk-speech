import React, { useRef, useEffect, useState } from 'react';
import { cn } from '../utils';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';

interface AudioPlayerProps {
  url: string | null;
  seekTo?: number | null;
  onTimeUpdate?: (time: number) => void;
  className?: string;
}

export const AudioPlayer: React.FC<AudioPlayerProps> = ({ url, seekTo, onTimeUpdate, className }) => {
  const audioRef = useRef<HTMLAudioElement>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);

  useEffect(() => {
    if (audioRef.current) {
      const audio = audioRef.current;
      const handleTimeUpdate = () => {
        setCurrentTime(audio.currentTime);
        onTimeUpdate?.(audio.currentTime);
      };
      const handleLoadedMetadata = () => setDuration(audio.duration);
      const handlePlay = () => setIsPlaying(true);
      const handlePause = () => setIsPlaying(false);

      audio.addEventListener('timeupdate', handleTimeUpdate);
      audio.addEventListener('loadedmetadata', handleLoadedMetadata);
      audio.addEventListener('play', handlePlay);
      audio.addEventListener('pause', handlePause);

      return () => {
        audio.removeEventListener('timeupdate', handleTimeUpdate);
        audio.removeEventListener('loadedmetadata', handleLoadedMetadata);
        audio.removeEventListener('play', handlePlay);
        audio.removeEventListener('pause', handlePause);
      };
    }
  }, [url, onTimeUpdate]);

  useEffect(() => {
    if (seekTo === null || seekTo === undefined) return;
    if (!audioRef.current) return;
    audioRef.current.currentTime = seekTo;
    setCurrentTime(seekTo);
  }, [seekTo]);

  const togglePlay = () => {
    if (audioRef.current) {
      if (isPlaying) audioRef.current.pause();
      else audioRef.current.play();
    }
  };

  const handleScrub = (e: React.ChangeEvent<HTMLInputElement>) => {
    const time = parseFloat(e.target.value);
    if (audioRef.current) {
      audioRef.current.currentTime = time;
      setCurrentTime(time);
    }
  };

  if (!url) return null;

  return (
    <div className={cn(
      "sticky bottom-0 left-0 right-0 bg-[var(--bg-card)] border-t border-[var(--line)] p-4 px-6 flex items-center gap-4 animate-fade-up",
      className
    )}>
      <audio ref={audioRef} src={url} />
      
      <Button 
        variant="primary" 
        size="icon" 
        className="w-9 h-9 rounded-full flex-shrink-0" 
        onClick={togglePlay}
      >
        <Icon name={isPlaying ? "pause" : "play"} size={16} fill="white" />
      </Button>

      <div className="flex-1 flex flex-col gap-1">
        <input
          type="range"
          min={0}
          max={duration || 100}
          step={0.1}
          value={currentTime}
          onChange={handleScrub}
          className="w-full h-1 bg-[var(--bg-soft)] rounded-full appearance-none cursor-pointer accent-[var(--primary)]"
          style={{
            background: `linear-gradient(to right, var(--primary) ${(currentTime / (duration || 1)) * 100}%, var(--bg-soft) 0%)`
          }}
        />
        <div className="flex justify-between items-center px-0.5">
          <span className="font-mono text-[11px] text-[var(--ink-3)]">
            {Math.floor(currentTime / 60)}:{Math.floor(currentTime % 60).toString().padStart(2, '0')} 
            <span className="mx-1 text-[var(--ink-4)]">/</span> 
            {Math.floor(duration / 60)}:{Math.floor(duration % 60).toString().padStart(2, '0')}
          </span>
        </div>
      </div>
    </div>
  );
};
