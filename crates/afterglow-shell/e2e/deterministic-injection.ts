( function () {

	/* Deterministic random */

	window.Math._random = window.Math.random;

	let seed = Math.PI / 4;
	window.Math.random = function () {

		const x = Math.sin( seed ++ ) * 10000;
		return x - Math.floor( x );

	};

	/* Deterministic timer */

	window.performance._now = performance.now;

	const now = () => 0; // frameId * 16;
	window.Date.now = now;
	window.Date.prototype.getTime = now;
	window.performance.now = now;

	/* Deterministic RAF and timer epoch */

	window._renderStarted = false;
	window._renderFinished = false;
	window.__setDeterministicTimerTime?.( 0 );
	const frameCallbacks = [];
	window.__deterministicFrameCount = () => frameCallbacks.length;

	window.requestAnimationFrame = function ( cb ) {

		if ( window._renderFinished === true || window._renderStarted === true ) return 0;
		frameCallbacks.push( cb );
		return frameCallbacks.length;

	};

	window.__runDeterministicFrame = function () {

		if ( window._renderFinished === true ) return;
		for ( const callback of frameCallbacks.splice( 0 ) ) callback( now() );
		window._renderFinished = true;

	};

	/* Semi-deterministic video */

	const play = HTMLVideoElement.prototype.play;

	HTMLVideoElement.prototype.play = async function () {

		play.call( this );
		this.addEventListener( 'timeupdate', () => this.pause() );

		function renew() {

			this.load();
			play.call( this );
			RAF( renew ); // eslint-disable-line no-undef

		}

		RAF( renew ); // eslint-disable-line no-undef

	};

	/* Additional variable for ~5 examples */

	window.TESTING = true;

}() );
