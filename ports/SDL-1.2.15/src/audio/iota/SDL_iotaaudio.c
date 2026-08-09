/*
    SDL - Simple DirectMedia Layer
    Copyright (C) 1997-2012 Sam Lantinga

    This library is free software; you can redistribute it and/or
    modify it under the terms of the GNU Lesser General Public
    License as published by the Free Software Foundation; either
    version 2.1 of the License, or (at your option) any later version.

    This library is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
    Lesser General Public License for more details.

    You should have received a copy of the GNU Lesser General Public
    License along with this library; if not, write to the Free Software
    Foundation, Inc., 51 Franklin St, Fifth Floor, Boston, MA  02110-1301  USA

    Sam Lantinga
    slouken@libsdl.org
*/
#include "SDL_config.h"

/* The iota SDL audio driver.
 *
 * There's no thread support in Iota/wasm, so we can't have a background
 * thread (not here or in the host) running audio. Instead we use a model
 * where audio is pumped as part of the event loop.
 * IOTAAUD_Pump prepares an audio buffer, then hands it off to the host
 * to do whatever it wants with it. */

#include "SDL.h"
#include "../SDL_audiomem.h"
#include "../SDL_audio_c.h"
#include "SDL_iotaaudio.h"

static SDL_AudioDevice *iota_audio_device;

IOTA_IMPORT(_iota_audio_init)    int  _iota_audio_init(int freq, int format, int channels, int samples, int size);
IOTA_IMPORT(_iota_audio_quit)    void _iota_audio_quit(void);
IOTA_IMPORT(_iota_audio_request) int  _iota_audio_request(void);
IOTA_IMPORT(_iota_audio_push)    void _iota_audio_push(Uint8 *buf, int len);

static int IOTAAUD_OpenAudio(_THIS, SDL_AudioSpec *spec);
static void IOTAAUD_CloseAudio(_THIS);

static int IOTAAUD_Available(void)
{
    return 1;
}

static void IOTAAUD_DeleteDevice(SDL_AudioDevice *device)
{
    SDL_free(device->hidden);
    SDL_free(device);
}

static SDL_AudioDevice *IOTAAUD_CreateDevice(int devindex)
{
    SDL_AudioDevice *device;

    /* Initialize all variables that we clean on shutdown */
    device = (SDL_AudioDevice *)SDL_malloc(sizeof(SDL_AudioDevice));
    if (device) {
        SDL_memset(device, 0, (sizeof *device));
        device->hidden = (struct SDL_PrivateAudioData *)
                SDL_malloc((sizeof *device->hidden));
    }
    if ((device == NULL) || (device->hidden == NULL)) {
        SDL_OutOfMemory();
        if ( device ) {
            SDL_free(device);
        }
        return(0);
    }
    SDL_memset(device->hidden, 0, (sizeof *device->hidden));

    /* Set the function pointers */
    device->OpenAudio = IOTAAUD_OpenAudio;
    device->CloseAudio = IOTAAUD_CloseAudio;

    device->free = IOTAAUD_DeleteDevice;

    return device;
}

AudioBootStrap IOTAAUD_bootstrap = {
    "iota", "SDL iota audio driver",
    IOTAAUD_Available, IOTAAUD_CreateDevice
};

static int IOTAAUD_OpenAudio(_THIS, SDL_AudioSpec *spec)
{
    if(iota_audio_device) {
        /* Disallow double-opens */
        return(-1);
    }
    /* Pass the spec unchanged, let the host deal with transforming the audio */
    this->hidden->mixbuf = (Uint8 *) SDL_AllocAudioMem(spec->size);
    if (this->hidden->mixbuf == NULL) {
        return(-1);
    }
    if (_iota_audio_init(spec->freq, spec->format, spec->channels, spec->samples, spec->size)) {
        SDL_FreeAudioMem(this->hidden->mixbuf);
        this->hidden->mixbuf = NULL;
        return(-1);
    }
    iota_audio_device = this;
    return(1);
}

static void IOTAAUD_CloseAudio(_THIS)
{
    if(!iota_audio_device) {
        return;
    }
    if (this->hidden->mixbuf != NULL) {
        SDL_FreeAudioMem(this->hidden->mixbuf);
        this->hidden->mixbuf = NULL;
    }
    _iota_audio_quit();
    iota_audio_device = NULL;
}

int IOTAAUD_Pump(void)
{
    SDL_AudioDevice *audio = iota_audio_device;
    if (!audio || !audio->opened) {
        return(0);
    }
    {
        int want = _iota_audio_request(); /* bytes the host wants */
        while (want >= (int)audio->spec.size) {
            Uint8 *stream = audio->hidden->mixbuf;
            int len = audio->spec.size;

            SDL_memset(stream, audio->spec.silence, len);
            if (!audio->paused) {
                audio->spec.callback(audio->spec.userdata, stream, len);
            }

            /* Hand the filled buffer to the host. */
            _iota_audio_push(stream, len);
            want -= len;
        }
    }
    return(0);
}

/* end of SDL_iotaaudio.c ... */
